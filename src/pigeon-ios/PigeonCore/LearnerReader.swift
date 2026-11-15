// The learner's page reader: finds the anchors in a photo, maps the page onto it, reads the code strip
// and how dark each circle is; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §4.

import Foundation

/// A greyscale picture, row by row from the top-left: 0 is black, 255 white.
public struct GrayImage: Sendable {
    public let width: Int
    public let height: Int
    public var pixels: [UInt8]

    public init(width: Int, height: Int, pixels: [UInt8]) {
        self.width = width
        self.height = height
        self.pixels = pixels
    }

    public init(width: Int, height: Int, fill: UInt8) {
        self.init(width: width, height: height, pixels: Array(repeating: fill, count: width * height))
    }

    /// The pixel nearest (`x`, `y`), clamped to the picture.
    public func at(_ x: Double, _ y: Double) -> UInt8 {
        let ix = min(max(Int(x.rounded()), 0), width - 1)
        let iy = min(max(Int(y.rounded()), 0), height - 1)
        return pixels[iy * width + ix]
    }
}

/// A point on the page (in mm) or in a picture (in pixels).
public struct PlanePoint: Equatable, Sendable {
    public let x: Double
    public let y: Double

    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

/// A perspective map between two planes, as a 3×3 matrix row by row.
public struct Homography: Sendable {
    public let m: [Double]

    public init(m: [Double]) { self.m = m }

    /// The map taking each of four `from` points to its `to` point, or nil if three are in a line.
    public init?(from: [PlanePoint], to: [PlanePoint]) {
        guard from.count == 4, to.count == 4 else { return nil }
        var a = [[Double]]()
        for (p, q) in zip(from, to) {
            a.append([p.x, p.y, 1, 0, 0, 0, -q.x * p.x, -q.x * p.y, q.x])
            a.append([0, 0, 0, p.x, p.y, 1, -q.y * p.x, -q.y * p.y, q.y])
        }
        for c in 0..<8 {
            let pivot = (c..<8).max { abs(a[$0][c]) < abs(a[$1][c]) }!
            guard abs(a[pivot][c]) > 1e-9 else { return nil }
            a.swapAt(c, pivot)
            for r in 0..<8 where r != c {
                let f = a[r][c] / a[c][c]
                for k in c..<9 { a[r][k] -= f * a[c][k] }
            }
        }
        m = (0..<8).map { a[$0][8] / a[$0][$0] } + [1]
    }

    public func map(_ p: PlanePoint) -> PlanePoint {
        let w = m[6] * p.x + m[7] * p.y + m[8]
        return PlanePoint(x: (m[0] * p.x + m[1] * p.y + m[2]) / w, y: (m[3] * p.x + m[4] * p.y + m[5]) / w)
    }

    /// The map back, or nil if this one flattens the plane.
    public var inverse: Homography? {
        let a = m
        let c = [
            a[4] * a[8] - a[5] * a[7], a[2] * a[7] - a[1] * a[8], a[1] * a[5] - a[2] * a[4],
            a[5] * a[6] - a[3] * a[8], a[0] * a[8] - a[2] * a[6], a[2] * a[3] - a[0] * a[5],
            a[3] * a[7] - a[4] * a[6], a[1] * a[6] - a[0] * a[7], a[0] * a[4] - a[1] * a[3],
        ]
        let det = a[0] * c[0] + a[1] * c[3] + a[2] * c[6]
        guard abs(det) > 1e-12 else { return nil }
        return Homography(m: c.map { $0 / det })
    }
}

/// A circle this dark or darker is marked; one at least `unclearDarkness` but lighter is unclear.
public let markedDarkness = 0.55
public let unclearDarkness = 0.20

/// What a photo of a page says: its form and page, each question's marked choices (0-based on the page),
/// the questions with a circle too light to call, and how dark each circle was, 0 to 1.
public struct PageReading: Equatable, Sendable {
    public let form: Int
    public let page: Int
    public let marks: [[Int]]
    public let unclear: [Int]
    public let darkness: [[Double]]

    public init(form: Int, page: Int, marks: [[Int]], unclear: [Int], darkness: [[Double]]) {
        self.form = form
        self.page = page
        self.marks = marks
        self.unclear = unclear
        self.darkness = darkness
    }
}

/// The level splitting dark pixels from light, by Otsu's method.
func otsu(_ image: GrayImage) -> UInt8 {
    var hist = [Int](repeating: 0, count: 256)
    for v in image.pixels { hist[Int(v)] += 1 }
    let total = Double(image.pixels.count)
    let sum = hist.enumerated().reduce(0.0) { $0 + Double($1.offset * $1.element) }
    var sumB = 0.0, wB = 0.0, best = -1.0, level = 127
    for t in 0..<256 {
        wB += Double(hist[t])
        if wB == 0 { continue }
        let wF = total - wB
        if wF == 0 { break }
        sumB += Double(t * hist[t])
        let between = wB * wF * pow(sumB / wB - (sum - sumB) / wF, 2)
        if between > best {
            best = between
            level = t
        }
    }
    return UInt8(level)
}

/// A solid dark blob: its pixel count and centre.
struct Blob {
    let area: Int
    let centre: PlanePoint
}

/// The squarish solid dark blobs in `image`, of a size an anchor could be.
func blobs(_ image: GrayImage) -> [Blob] {
    let w = image.width, h = image.height
    let level = otsu(image)
    var seen = [Bool](repeating: false, count: w * h)
    let side = Double(min(w, h))
    var out: [Blob] = []
    var stack: [Int] = []
    for start in 0..<(w * h) where !seen[start] && image.pixels[start] <= level {
        seen[start] = true
        stack.append(start)
        var n = 0, sx = 0, sy = 0, x0 = w, x1 = 0, y0 = h, y1 = 0
        while let i = stack.popLast() {
            let x = i % w, y = i / w
            n += 1
            sx += x
            sy += y
            x0 = min(x0, x)
            x1 = max(x1, x)
            y0 = min(y0, y)
            y1 = max(y1, y)
            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
            where nx >= 0 && nx < w && ny >= 0 && ny < h {
                let j = ny * w + nx
                if !seen[j] && image.pixels[j] <= level {
                    seen[j] = true
                    stack.append(j)
                }
            }
        }
        let bw = Double(x1 - x0 + 1), bh = Double(y1 - y0 + 1)
        guard bw >= side * 0.01, bh >= side * 0.01, bw <= side * 0.12, bh <= side * 0.12,
            bw / bh >= 0.5, bw / bh <= 2, Double(n) / (bw * bh) >= 0.55
        else { continue }
        out.append(Blob(area: n, centre: PlanePoint(x: Double(sx) / Double(n), y: Double(sy) / Double(n))))
    }
    return out
}

/// The anchors' centres in `image`, in the page's order (top-left, top-right, bottom-left,
/// bottom-right) whichever way up it was photographed, or nil if four cannot be found.
public func findAnchors(_ image: GrayImage) -> [PlanePoint]? {
    let all = blobs(image)
    guard let biggest = all.map(\.area).max() else { return nil }
    let big = all.filter { Double($0.area) >= Double(biggest) * 0.25 }
    let w = Double(image.width), h = Double(image.height)
    // The picture's corners, clockwise from its top-left.
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    var picked: [Blob] = []
    for (cx, cy) in corners {
        guard
            let b = big.min(by: {
                hypot($0.centre.x - cx, $0.centre.y - cy) < hypot($1.centre.x - cx, $1.centre.y - cy)
            })
        else { return nil }
        if picked.contains(where: { $0.centre == b.centre }) { return nil }
        picked.append(b)
    }
    // The larger top-left anchor says which corner is the page's top-left.
    let top = picked.indices.max { picked[$0].area < picked[$1].area }!
    let cw = (0..<4).map { picked[(top + $0) % 4].centre }
    return [cw[0], cw[1], cw[3], cw[2]]
}

/// Each anchor's centre on the page, in the order `findAnchors` gives them.
public let anchorCentres = LearnerPage.anchors.map { PlanePoint(x: $0.x + $0.w / 2, y: $0.y + $0.h / 2) }

/// The mean darkness, 0 to 1, of `image` at the page points `points`, between `black` and `white`.
func darkness(
    _ image: GrayImage, _ page: Homography, _ points: [PlanePoint], black: Double, white: Double
) -> Double {
    let sum = points.reduce(0.0) { acc, p in
        let q = page.map(p)
        let v = Double(image.at(q.x, q.y))
        return acc + min(max((white - v) / (white - black), 0), 1)
    }
    return sum / Double(points.count)
}

/// Points spread over a circle of `radius` mm about (`x`, `y`).
func disc(_ x: Double, _ y: Double, _ radius: Double) -> [PlanePoint] {
    var out: [PlanePoint] = []
    for i in -3...3 {
        for j in -3...3 where i * i + j * j <= 9 {
            out.append(PlanePoint(x: x + Double(i) * radius / 3, y: y + Double(j) * radius / 3))
        }
    }
    return out
}

/// Points spread over the middle `share` of `box`.
func grid(_ box: PageBox, _ share: Double) -> [PlanePoint] {
    var out: [PlanePoint] = []
    for i in 0..<5 {
        for j in 0..<5 {
            out.append(
                PlanePoint(
                    x: box.x + box.w * (0.5 + share * (Double(i) / 4 - 0.5)),
                    y: box.y + box.h * (0.5 + share * (Double(j) / 4 - 0.5))))
        }
    }
    return out
}

/// Reads a photo of a printed page: `circles` gives the circles a form's page was printed with, or nil
/// for one this phone did not print. Nil when the anchors, the code or the circles cannot be read.
public func readPage(_ image: GrayImage, circles: (Int, Int) -> [PlacedCircle]?) -> PageReading? {
    guard let found = findAnchors(image), let page = Homography(from: anchorCentres, to: found) else {
        return nil
    }
    var blacks: [Double] = [], whites: [Double] = []
    for a in LearnerPage.anchors {
        for p in grid(a, 0.5) {
            let q = page.map(p)
            blacks.append(Double(image.at(q.x, q.y)))
        }
        let cx = a.x + a.w / 2, cy = a.y + a.h / 2, d = a.w / 2 + 1.5
        for (dx, dy) in [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d)] {
            let q = page.map(PlanePoint(x: cx + dx, y: cy + dy))
            whites.append(Double(image.at(q.x, q.y)))
        }
    }
    let black = blacks.reduce(0, +) / Double(blacks.count)
    let white = whites.sorted()[whites.count / 2]
    guard white - black >= 40 else { return nil }
    let bits = (0..<12).map {
        darkness(image, page, grid(LearnerPage.codeCell($0), 0.6), black: black, white: white) >= 0.5
    }
    guard let code = decodeCode(bits), let list = circles(code.form, code.page), !list.isEmpty else {
        return nil
    }
    let first = list.map(\.question).min()!
    let count = list.map(\.question).max()! - first + 1
    var dark = [[Double]](repeating: [0, 0, 0, 0], count: count)
    for c in list where c.choice >= 0 && c.choice < 4 {
        dark[c.question - first][c.choice] = darkness(
            image, page, disc(c.x, c.y, LearnerPage.circleRadius * 0.75), black: black, white: white)
    }
    let marks = dark.map { row in row.indices.filter { row[$0] >= markedDarkness } }
    let unclear = dark.indices.filter { q in
        dark[q].contains { $0 >= unclearDarkness && $0 < markedDarkness }
    }
    return PageReading(form: code.form, page: code.page, marks: marks, unclear: unclear, darkness: dark)
}
