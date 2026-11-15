// PigeonCore's scan countdown: 3-2-1 on a steady page, cancelled when the page is lost or moves.

import Foundation
import Testing

import PigeonCore

/// Four anchors of a page in a 1080-pixel-high picture, shifted by (`dx`, `dy`).
private func page(_ dx: Double = 0, _ dy: Double = 0) -> [PlanePoint] {
    [
        PlanePoint(x: 100 + dx, y: 80 + dy), PlanePoint(x: 900 + dx, y: 80 + dy),
        PlanePoint(x: 100 + dx, y: 1000 + dy), PlanePoint(x: 900 + dx, y: 1000 + dy),
    ]
}

/// The countdown's step after each frame: its anchors and its time in seconds.
private func run(_ frames: [([PlanePoint]?, Double)], _ c: inout ScanCountdown) -> [ScanCountdown.Step] {
    frames.map { c.update(anchors: $0.0, side: 1080, at: $0.1) }
}

@Suite struct LearnerCountdownTests {
    @Test func countsThreeTwoOneThenCaptures() {
        var c = ScanCountdown()
        let steps = run(
            [(nil, 0), (page(), 1), (page(), 1.5), (page(), 2.2), (page(), 3.1), (page(), 4), (page(), 4.1)], &c)
        #expect(steps == [.waiting, .counting(3), .counting(3), .counting(2), .counting(1), .capture, .counting(3)])
    }

    @Test func aLostAnchorCancelsAndRestarts() {
        var c = ScanCountdown()
        let three = Array(page().prefix(3))
        let steps = run(
            [(page(), 0), (page(), 1.5), (nil, 1.6), (three, 1.7), (page(), 2), (page(), 4.5), (page(), 5)], &c)
        #expect(steps == [.counting(3), .counting(2), .waiting, .waiting, .counting(3), .counting(1), .capture])
    }

    @Test func aMovingPageRestartsButJitterDoesNot() {
        var c = ScanCountdown()
        let steps = run(
            [(page(), 0), (page(8, -6), 1.2), (page(68, -6), 1.4), (page(70, -4), 2.5), (page(70, -4), 4.4)], &c)
        #expect(steps == [.counting(3), .counting(2), .counting(3), .counting(2), .capture])
    }

    @Test func resetCancelsTheCountdown() {
        var c = ScanCountdown()
        let before = run([(page(), 0), (page(), 2.5)], &c)
        c.reset()
        let after = run([(page(), 2.6)], &c)
        #expect(before == [.counting(3), .counting(1)])
        #expect(after == [.counting(3)])
    }
}
