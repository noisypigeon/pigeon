// The learner's printed page: its fixed geometry and the layout of 25 questions, keeping each circle's centre;
// see .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §3 and its 2026-11-15 amendment.

import Foundation

/// A rectangle on the page, in millimetres from its top-left.
public struct PageBox: Codable, Equatable, Sendable {
    public let x: Double
    public let y: Double
    public let w: Double
    public let h: Double

    public init(x: Double, y: Double, w: Double, h: Double) {
        self.x = x
        self.y = y
        self.w = w
        self.h = h
    }
}

/// The printed page's fixed geometry: US Letter, in millimetres.
public enum LearnerPage {
    public static let width = 215.9
    public static let height = 279.4
    public static let top = 35.0
    public static let bottom = 262.0
    public static let columns = [18.0, 112.0]
    public static let columnWidth = 86.0
    public static let pair = 42.0
    public static let choiceWidth = 34.0
    public static let circleRadius = 1.85
    /// Top-left (the larger, which says which way is up), top-right, bottom-left, bottom-right.
    public static let anchors = [
        PageBox(x: 8, y: 8, w: 8, h: 8),
        PageBox(x: 201.9, y: 9, w: 6, h: 6),
        PageBox(x: 9, y: 264.4, w: 6, h: 6),
        PageBox(x: 201.9, y: 264.4, w: 6, h: 6),
    ]
    public static let codeFrame = PageBox(x: 166.5, y: 10.1, w: 28.8, h: 5)

    /// Code bit `b`'s cell, 0 to 11; the page's two bits sit a little apart from the form's ten.
    public static func codeCell(_ b: Int) -> PageBox {
        PageBox(x: 167 + Double(b) * 2.3 + (b >= 10 ? 0.6 : 0), y: 10.6, w: 1.8, h: 4)
    }
}

/// A line of text to draw: `y` is its baseline; `trailing` text ends at `x` instead of starting there.
public struct PlacedText: Equatable, Sendable {
    public let text: String
    public let x: Double
    public let y: Double
    public let size: Double
    public let bold: Bool
    public let trailing: Bool

    public init(text: String, x: Double, y: Double, size: Double, bold: Bool, trailing: Bool) {
        self.text = text
        self.x = x
        self.y = y
        self.size = size
        self.bold = bold
        self.trailing = trailing
    }
}

/// A circle's centre: `question` is its index on the form (0-based), `choice` 0 for A to 3 for D.
public struct PlacedCircle: Codable, Equatable, Sendable {
    public let question: Int
    public let choice: Int
    public let x: Double
    public let y: Double

    public init(question: Int, choice: Int, x: Double, y: Double) {
        self.question = question
        self.choice = choice
        self.x = x
        self.y = y
    }
}

/// One page laid out: the scale its text was set at, every line of text, and every circle.
public struct PageLayout: Equatable, Sendable {
    public let scale: Double
    public let texts: [PlacedText]
    public let circles: [PlacedCircle]

    public init(scale: Double, texts: [PlacedText], circles: [PlacedCircle]) {
        self.scale = scale
        self.texts = texts
        self.circles = circles
    }
}

/// Wraps `text` set at `size` mm (bold or not) into lines no wider than `width` mm.
public typealias Wrap = (_ text: String, _ size: Double, _ bold: Bool, _ width: Double) -> [String]

private struct Measured {
    let qs: Double
    let qlh: Double
    let cs: Double
    let clh: Double
    let qLines: [String]
    let choices: [[String]]
    let rows: [Double]
    let head: Double
    var h: Double { head + rows[0] + rows[1] }
}

private func measure(_ q: Question, _ k: Double, _ wrap: Wrap) -> Measured {
    let qs = 2.5 * k, qlh = 2.95 * k, cs = 2.3 * k, clh = 2.7 * k
    let qLines = wrap(q.question, qs, true, LearnerPage.columnWidth - 6.5)
    let ch = q.choices.map { wrap($0, cs, false, LearnerPage.choiceWidth) }
    func row(_ r: Int) -> Double {
        4.1 * k + Double(max(ch[2 * r].count, ch[2 * r + 1].count, 1) - 1) * clh
    }
    let head = Double(qLines.count) * qlh + 0.7 * k
    return Measured(
        qs: qs, qlh: qlh, cs: cs, clh: clh, qLines: qLines, choices: ch, rows: [row(0), row(1)],
        head: head)
}

/// Lays out up to 25 questions in two columns, each with its choices in a 2 × 2 grid, at the
/// largest scale that fits; spare height is shared out as even gaps. `first` is the form index of
/// the page's first question.
public func layoutPage(_ questions: [Question], first: Int, wrap: Wrap) -> PageLayout {
    let split = (questions.count + 1) / 2
    let cols = [Array(questions.prefix(split)), Array(questions.dropFirst(split))]
    let scales = [1.0, 0.96, 0.92, 0.88, 0.85, 0.82, 0.79, 0.76]
    let room = LearnerPage.bottom - LearnerPage.top
    for k in scales {
        let ms = cols.map { $0.map { measure($0, k, wrap) } }
        let need = ms.map { c in c.reduce(0) { $0 + $1.h } + Double(max(c.count - 1, 0)) * 2 * k }
        guard (need.max() ?? 0) <= room || k == scales.last else { continue }
        var texts: [PlacedText] = []
        var circles: [PlacedCircle] = []
        for (ci, c) in ms.enumerated() {
            let gap = c.count > 1 ? min(6, (room - need[ci]) / Double(c.count - 1) + 2 * k) : 0
            let x = LearnerPage.columns[ci]
            var y = LearnerPage.top
            for (j, m) in c.enumerated() {
                let index = first + (ci == 0 ? j : split + j)
                let y0 = y + m.qs * 0.8
                texts.append(
                    PlacedText(
                        text: String(index + 1), x: x + 4.6, y: y0, size: 2.7 * k, bold: true,
                        trailing: true))
                for (l, line) in m.qLines.enumerated() {
                    texts.append(
                        PlacedText(
                            text: line, x: x + 6.5, y: y0 + Double(l) * m.qlh, size: m.qs, bold: true,
                            trailing: false))
                }
                var ry = y + m.head
                for r in 0..<2 {
                    let cy = ry + 2.05 * k
                    for side in 0..<2 {
                        let choice = r * 2 + side
                        let bx = x + 8.6 + Double(side) * LearnerPage.pair
                        circles.append(PlacedCircle(question: index, choice: choice, x: bx, y: cy))
                        for (l, line) in m.choices[choice].enumerated() {
                            texts.append(
                                PlacedText(
                                    text: line, x: bx + 3, y: cy + 0.8 * k + Double(l) * m.clh,
                                    size: m.cs, bold: false, trailing: false))
                        }
                    }
                    ry += m.rows[r]
                }
                y += m.h + gap
            }
        }
        return PageLayout(scale: k, texts: texts, circles: circles)
    }
    return PageLayout(scale: 1, texts: [], circles: [])
}
