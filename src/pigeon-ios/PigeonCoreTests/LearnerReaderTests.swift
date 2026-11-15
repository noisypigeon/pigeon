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

/// A page of form 17, page 1, photographed through `view`; `fills` as `photo`'s.
private func photo(view: Homography, width: Int, height: Int, fills: [Int: Double]) -> GrayImage {
    photo(view: view, width: width, height: height, form: 17, page: 1, circles: circles, fills: fills)
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
