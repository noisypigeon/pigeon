// PigeonCore's learner page: 25 questions fit inside the margins, and long text shrinks to fit.

import Foundation
import Testing

import PigeonCore

/// 25 short questions, as a typical page holds.
private func sampleQuestions() -> [Question] {
    (1...25).map {
        Question(
            id: $0, topic: "Parking", question: "Question \($0) about parking near a hydrant?",
            choices: ["One", "Two", "Three", "Four"], answer: "A", explanation: "", source: "")
    }
}

/// A stand-in for real text measurement: each character is half the text's size wide.
private func fakeWrap(_ text: String, _ size: Double, _ bold: Bool, _ width: Double) -> [String] {
    let perLine = max(1, Int(width / (size * 0.5)))
    var lines: [String] = []
    var cur = ""
    for w in text.split(separator: " ") {
        let t = cur.isEmpty ? String(w) : cur + " " + w
        if t.count <= perLine || cur.isEmpty { cur = t } else {
            lines.append(cur)
            cur = String(w)
        }
    }
    if !cur.isEmpty { lines.append(cur) }
    return lines
}

@Suite struct LearnerLayoutTests {
    @Test func pageHoldsTwentyFiveQuestionsInsideItsMargins() {
        let qs = sampleQuestions()
        let page = layoutPage(qs, first: 0, wrap: fakeWrap)
        #expect(page.scale == 1)
        #expect(page.circles.count == 100)
        #expect(page.circles.allSatisfy { $0.y >= LearnerPage.top && $0.y <= LearnerPage.bottom })
        #expect(page.circles.allSatisfy { $0.x >= 18 && $0.x <= LearnerPage.width - 18 })
        for i in 0..<25 {
            #expect(page.circles.filter { $0.question == i }.map(\.choice) == [0, 1, 2, 3])
        }
    }

    @Test func secondPageNumbersFollowOn() {
        let qs = sampleQuestions()
        let page = layoutPage(qs, first: 25, wrap: fakeWrap)
        #expect(page.circles.map(\.question).min() == 25)
        #expect(page.circles.map(\.question).max() == 49)
        #expect(page.texts.first?.text == "26")
    }

    @Test func longTextShrinksToFit() {
        let long = String(repeating: "word ", count: 20)
        let choice = String(repeating: "abcd ", count: 8)
        let qs = (1...25).map {
            Question(
                id: $0, topic: "Parking", question: long, choices: [choice, choice, choice, choice],
                answer: "A", explanation: "", source: "")
        }
        let page = layoutPage(qs, first: 0, wrap: fakeWrap)
        #expect(page.scale < 1)
        #expect(page.circles.allSatisfy { $0.y <= LearnerPage.bottom })
    }
}
