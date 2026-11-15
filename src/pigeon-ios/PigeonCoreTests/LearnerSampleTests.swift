// PigeonCore's learner sample page: plainly a sample, read back from its filled-in printing, and never kept.

import Foundation
import Testing

import PigeonCore

/// The filled-in sample's circles, as indices into the sample layout's circles, in pen.
private func filledIn() -> [Int: Double] {
    var out: [Int: Double] = [:]
    for (i, c) in sampleLayout.circles.enumerated() where sampleFills.contains(c) { out[i] = 1 }
    return out
}

/// A ledger holding one real form, its page's circles and one scored attempt.
private func realLedger() -> LearnerLedger {
    var ledger = LearnerLedger()
    let form = Form(number: 17, questionIDs: [1, 2], key: [0, 1])
    let circles = [PlacedCircle(question: 0, choice: 0, x: 30, y: 40)]
    ledger.add(form, circles: [circles])
    _ = ledger.record(form: 17, page: 0, marks: [[0], [1]], id: "a1", date: 1)
    return ledger
}

@Suite struct LearnerSampleTests {
    @Test func theSampleIsPlainlyASample() {
        #expect(sampleQuestions.count == questionsPerPage)
        for (i, q) in sampleQuestions.enumerated() {
            #expect(q.question.hasPrefix("Sample \(i + 1): "))
            #expect(q.topic == "Sample")
        }
        #expect(sampleForm.number == sampleFormNumber)
        #expect(isSample(sampleForm.number))
        let code = decodeCode(codeBits(form: sampleFormNumber, page: 0))
        #expect(code?.form == sampleFormNumber)
        #expect(code?.page == 0)
    }

    @Test func theSampleNeverWrapsSoItsCirclesAreFixed() {
        #expect(sampleLayout.circles.count == questionsPerPage * 4)
        let lines = sampleLayout.texts.filter { $0.bold && !$0.trailing }
        #expect(lines.count == questionsPerPage)
        #expect(sampleFills.count == 25)
    }

    @Test func realFormsNeverTakeTheSampleNumber() {
        var ledger = LearnerLedger()
        ledger.add(Form(number: 1022, questionIDs: [1], key: [0]), circles: [])
        #expect(ledger.nextFormNumber == 0)
        let before = ledger
        ledger.add(sampleForm, circles: [sampleLayout.circles])
        #expect(ledger == before)
    }

    @Test func theFilledInSampleReadsBackAndScores() {
        let ledger = realLedger()
        let view = camera(scale: 3.4, degrees: 9, tilt: 0.08, margin: 90)
        let image = photo(
            view: view, width: 1000, height: 1200, form: sampleFormNumber, page: 0,
            circles: sampleLayout.circles, fills: filledIn())
        let reading = readPage(image) { ledger.pageCircles(form: $0, page: $1) }
        #expect(reading?.form == sampleFormNumber)
        #expect(reading?.page == 0)
        #expect(reading?.marks == sampleMarks)
        #expect(reading?.unclear == [])
        var after = ledger
        let attempt = after.record(
            form: sampleFormNumber, page: 0, marks: reading?.marks ?? [], id: "s", date: 2)
        #expect(attempt?.correct == 21)
        #expect(attempt?.results[16] == .double)
        #expect(attempt?.results[22] == .blank)
        #expect(attempt.map { hints(attempt: $0, bank: sampleQuestions).count } == 4)
        #expect(after == ledger)
    }

    @Test func aSampleIsNeverKept() throws {
        var ledger = realLedger()
        let before = ledger
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let saved = try encoder.encode(ledger)
        let attempt = ledger.record(form: sampleFormNumber, page: 0, marks: sampleMarks, id: "s", date: 3)
        #expect(attempt?.results.count == questionsPerPage)
        #expect(ledger == before)
        #expect(try encoder.encode(ledger) == saved)
        #expect(ledger.attempts.count == 1)
        #expect(ledger.best == 2)
        #expect(ledger.forms.map(\.number) == [17])
        #expect(ledger.circles.keys.sorted() == ["17-0"])
        let pageTwo = ledger.record(form: sampleFormNumber, page: 1, marks: [], id: "s", date: 3)
        #expect(pageTwo == nil)
        _ = ledger.record(form: 17, page: 0, marks: [[1], [1]], id: "a2", date: 4)
        #expect(ledger.attempts.count == 2)
    }
}
