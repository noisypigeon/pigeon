// The scan's countdown: 3-2-1 on a steady page, restarted when it moves or is lost; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md, its countdown amendment.

import Foundation

/// The camera's countdown, fed each frame's anchors.
public struct ScanCountdown: Sendable {
    /// What the camera does after a frame.
    public enum Step: Equatable, Sendable {
        /// No page in view: no countdown.
        case waiting
        /// Counting down: the whole seconds left, 3 to 1.
        case counting(Int)
        /// Read this frame.
        case capture
    }

    public let seconds: Double
    /// How far an anchor may move between frames, as a share of the picture's shorter side.
    public let tolerance: Double
    private var start: Double?
    private var last: [PlanePoint]?

    public init(seconds: Double = 3, tolerance: Double = 0.02) {
        self.seconds = seconds
        self.tolerance = tolerance
    }

    /// The step after a frame at `time` (seconds) whose anchors were `anchors`, nil when not all four
    /// were found; `side` is the picture's shorter side in pixels.
    public mutating func update(anchors: [PlanePoint]?, side: Double, at time: Double) -> Step {
        guard let anchors, anchors.count == 4 else {
            reset()
            return .waiting
        }
        if let start, let last, steady(last, anchors, side) {
            self.last = anchors
            let left = seconds - (time - start)
            if left <= 0 {
                reset()
                return .capture
            }
            return .counting(Int(left.rounded(.up)))
        }
        start = time
        last = anchors
        return .counting(Int(seconds.rounded(.up)))
    }

    /// Cancels the countdown; the next frame with a page starts it again.
    public mutating func reset() {
        start = nil
        last = nil
    }

    private func steady(_ a: [PlanePoint], _ b: [PlanePoint], _ side: Double) -> Bool {
        zip(a, b).allSatisfy { hypot($0.x - $1.x, $0.y - $1.y) <= tolerance * side }
    }
}
