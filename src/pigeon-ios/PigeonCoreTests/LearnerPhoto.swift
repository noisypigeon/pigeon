// Pages drawn as a camera would see them, for the learner reader's tests.

import Foundation

import PigeonCore

let paper = 236.0, ink = 22.0

func pt(_ x: Double, _ y: Double) -> PlanePoint { PlanePoint(x: x, y: y) }

/// Page `page` of form `form`, printed with `circles`, photographed through `view` (page mm to picture
/// pixels): `fills` gives each filled circle's index in `circles` and how dark its pencil is, 0 to 1.
func photo(
    view: Homography, width: Int, height: Int, form: Int, page: Int, circles: [PlacedCircle],
    fills: [Int: Double]
) -> GrayImage {
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
        let quad: [PlanePoint] = [pt(box.x, box.y), pt(right, box.y), pt(box.x, bottom), pt(right, bottom)]
        let corners: [PlanePoint] = quad.map { view.map($0) }
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
    for (b, on) in codeBits(form: form, page: page).enumerated() where on {
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

func inside(_ b: PageBox, _ p: PlanePoint) -> Bool {
    let right: Double = b.x + b.w
    let bottom: Double = b.y + b.h
    return p.x >= b.x && p.x <= right && p.y >= b.y && p.y <= bottom
}

/// A circle's shade at `p`: its pencil fill inside, its printed outline on the ring, else nothing.
func shade(_ p: PlanePoint, _ c: PlacedCircle, _ r: Double, _ pencil: Double?) -> Double? {
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
func camera(scale: Double, degrees: Double, tilt: Double, margin: Double) -> Homography {
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
