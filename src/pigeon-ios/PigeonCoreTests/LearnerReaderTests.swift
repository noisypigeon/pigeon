// PigeonCore's learner reader: pages drawn as a camera would see them, straight, at an angle and upside
// down, read back to the marks they were filled with.

import Foundation
import Testing

import PigeonCore

/// 25 questions in two columns, four circles each, as a page lays them out.
private let circles: [PlacedCircle] = makeCircles()

private func makeCircles() -> [PlacedCircle] {
    var out: [PlacedCircle] = []
    for q in 0..<25 {
        let column: Double = q < 13 ? 0 : 94
        let row: Double = Double(q % 13) * 16.5
        for c in 0..<4 {
            let x: Double = 24 + column + Double(c) * 8.5
            out.append(PlacedCircle(question: 25 + q, choice: c, x: x, y: 40 + row))
        }
    }
    return out
}

private let paper = 236.0, ink = 22.0

private func pt(_ x: Double, _ y: Double) -> PlanePoint { PlanePoint(x: x, y: y) }

/// A page of form 17, page 1, photographed through `view` (page mm to picture pixels): `fills` gives
/// each filled circle's index in `circles` and how dark its pencil is, 0 to 1.
private func photo(view: Homography, width: Int, height: Int, fills: [Int: Double]) -> GrayImage {
    var image = GrayImage(width: width, height: height, fill: UInt8(paper))
    var rng = Mulberry32(seed: 7)
    for i in image.pixels.indices {
        let v: Double = paper - 14 * rng.next()
        image.pixels[i] = UInt8(v)
    }
    let back = view.inverse!
    func paint(_ box: PageBox, _ shade: (PlanePoint) -> Double?) {
        let right: Double = box.x + box.w
        let bottom: Double = box.y + box.h
        let page: [PlanePoint] = [pt(box.x, box.y), pt(right, box.y), pt(box.x, bottom), pt(right, bottom)]
        let corners: [PlanePoint] = page.map { view.map($0) }
        let xs: [Double] = corners.map(\.x)
        let ys: [Double] = corners.map(\.y)
        let x0: Int = max(Int(xs.min()!) - 1, 0)
        let x1: Int = min(Int(xs.max()!) + 1, width - 1)
        let y0: Int = max(Int(ys.min()!) - 1, 0)
        let y1: Int = min(Int(ys.max()!) + 1, height - 1)
        guard x0 <= x1, y0 <= y1 else { return }
        for y in y0...y1 {
            for x in x0...x1 {
                let p = back.map(PlanePoint(x: Double(x), y: Double(y)))
                if let v = shade(p) { image.pixels[y * width + x] = UInt8(v) }
            }
        }
    }
    for a in LearnerPage.anchors { paint(a) { inside(a, $0) ? ink : nil } }
    for (b, on) in codeBits(form: 17, page: 1).enumerated() where on {
        let cell = LearnerPage.codeCell(b)
        paint(cell) { inside(cell, $0) ? ink : nil }
    }
    let r: Double = LearnerPage.circleRadius
    for (i, c) in circles.enumerated() {
        let box = PageBox(x: c.x - r - 0.5, y: c.y - r - 0.5, w: 2 * r + 1, h: 2 * r + 1)
        let pencil: Double? = fills[i]
        paint(box) { p in shade(p, c, r, pencil) }
    }
    return image
}

private func inside(_ b: PageBox, _ p: PlanePoint) -> Bool {
    let right: Double = b.x + b.w
    let bottom: Double = b.y + b.h
    return p.x >= b.x && p.x <= right && p.y >= b.y && p.y <= bottom
}

/// A circle's shade at `p`: its pencil fill inside, its printed outline on the ring, else nothing.
private func shade(_ p: PlanePoint, _ c: PlacedCircle, _ r: Double, _ pencil: Double?) -> Double? {
    let d: Double = hypot(p.x - c.x, p.y - c.y)
    if let pencil, d < r - 0.1 {
        let v: Double = paper - pencil * (paper - ink)
        return v
    }
    let ring: Double = abs(d - r)
    return ring < 0.15 ? 90 : nil
}

/// The view of a page `scale` pixels per mm, turned `degrees`, its far edge `tilt` narrower, shifted
/// by `margin`.
private func camera(scale: Double, degrees: Double, tilt: Double, margin: Double) -> Homography {
    let w = LearnerPage.width, h = LearnerPage.height
    let page = [pt(0, 0), pt(w, 0), pt(0, h), pt(w, h)]
    let t = degrees * .pi / 180
    let cx: Double = w * scale / 2 + margin
    let cy: Double = h * scale / 2 + margin
    let c: Double = cos(t), s: Double = sin(t)
    let flat = [pt(tilt * w, 0), pt(w - tilt * w, 0), pt(0, h), pt(w, h)].map { p -> PlanePoint in
        let dx: Double = p.x * scale - w * scale / 2
        let dy: Double = p.y * scale - h * scale / 2
        let x: Double = cx + dx * c - dy * s
        let y: Double = cy + dx * s + dy * c
        return pt(x, y)
    }
    return Homography(from: page, to: flat)!
}

private func lookup(_ form: Int, _ page: Int) -> [PlacedCircle]? {
    form == 17 && page == 1 ? circles : nil
}

/// A page's 25 questions' marks: `filled` names the marked ones, the rest are blank.
private func marked(_ filled: [Int: [Int]]) -> [[Int]] {
    var out: [[Int]] = Array(repeating: [], count: 25)
    for (q, choices) in filled { out[q] = choices }
    return out
}

/// Question q's circle for choice c, as an index into `circles`.
private func at(_ q: Int, _ c: Int) -> Int { q * 4 + c }

@Suite struct LearnerReaderTests {
    @Test func homographyMapsItsFourPointsAndBack() {
        let from = [pt(0, 0), pt(10, 0), pt(0, 20), pt(10, 20)]
        let to = [pt(5, 7), pt(40, 3), pt(2, 80), pt(47, 90)]
        let h = Homography(from: from, to: to)!
        for (p, q) in zip(from, to) {
            let m = h.map(p), back = h.inverse!.map(q)
            let ahead: Double = hypot(m.x - q.x, m.y - q.y)
            let behind: Double = hypot(back.x - p.x, back.y - p.y)
            #expect(ahead < 1e-6)
            #expect(behind < 1e-6)
        }
        #expect(Homography(from: Array(repeating: from[0], count: 4), to: to) == nil)
    }

    @Test func readsAStraightPage() {
        let view = camera(scale: 4, degrees: 0, tilt: 0, margin: 20)
        let fills = [at(0, 1): 1.0, at(5, 3): 0.9, at(12, 0): 0.8, at(24, 2): 1.0]
        let reading = readPage(photo(view: view, width: 904, height: 1158, fills: fills), circles: lookup)!
        #expect(reading.form == 17)
        #expect(reading.page == 1)
        #expect(reading.marks == marked([0: [1], 5: [3], 12: [0], 24: [2]]))
        #expect(reading.unclear.isEmpty)
    }

    @Test func readsAPagePhotographedAtAnAngle() {
        let view = camera(scale: 3.4, degrees: 9, tilt: 0.08, margin: 90)
        let fills = [at(2, 0): 1.0, at(7, 1): 0.85, at(7, 2): 0.85, at(20, 3): 1.0]
        let reading = readPage(photo(view: view, width: 1000, height: 1200, fills: fills), circles: lookup)!
        #expect(reading.form == 17)
        #expect(reading.page == 1)
        #expect(reading.marks == marked([2: [0], 7: [1, 2], 20: [3]]))
    }

    @Test func readsAPageUpsideDown() {
        let view = camera(scale: 3.6, degrees: 180, tilt: 0.04, margin: 30)
        let fills = [at(3, 2): 1.0, at(17, 1): 1.0]
        let reading = readPage(photo(view: view, width: 900, height: 1120, fills: fills), circles: lookup)!
        #expect(reading.form == 17)
        #expect(reading.page == 1)
        #expect(reading.marks == marked([3: [2], 17: [1]]))
    }

    @Test func aLightMarkIsUnclear() {
        let view = camera(scale: 4, degrees: 2, tilt: 0, margin: 20)
        let fills = [at(4, 0): 0.35, at(9, 3): 1.0]
        let reading = readPage(photo(view: view, width: 904, height: 1158, fills: fills), circles: lookup)!
        #expect(reading.marks == marked([9: [3]]))
        #expect(reading.unclear == [4])
    }

    @Test func aBlankPictureOrAnUnknownFormReadsAsNothing() {
        #expect(readPage(GrayImage(width: 300, height: 400, fill: 230), circles: lookup) == nil)
        let view = camera(scale: 4, degrees: 0, tilt: 0, margin: 20)
        let image = photo(view: view, width: 904, height: 1158, fills: [:])
        #expect(readPage(image, circles: { _, _ in nil }) == nil)
    }
}
