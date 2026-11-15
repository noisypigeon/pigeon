// The learner's practice on this iPhone: the bank, and PigeonCore's ledger of printed forms, each page's
// circles and scored pages, kept until the learner's instance holds them; the sample is never saved; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md step 2 and its sample amendment.

import Foundation
import PigeonCore

@MainActor
final class LearnerStore: ObservableObject {
    @Published private(set) var ledger = LearnerLedger()
    let bank: [Question]

    private let folder: URL

    init() {
        let url = Bundle.main.url(forResource: "questions", withExtension: "json")
        bank = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? decodeBank($0) } ?? []
        folder = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("learner", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        ledger = LearnerLedger(
            forms: load("forms.json") ?? [], attempts: load("attempts.json") ?? [],
            circles: load("circles.json") ?? [:])
    }

    var forms: [Form] { ledger.forms }
    var attempts: [Attempt] { ledger.attempts }

    /// The best score so far, or nil before the first.
    var best: Int? { ledger.best }

    /// Form `number`, the sample's included.
    func form(_ number: Int) -> Form? { ledger.form(number) }

    /// The questions an attempt's IDs come from.
    func questions(for attempt: Attempt) -> [Question] { isSample(attempt.form) ? sampleQuestions : bank }

    /// A new form of `count` questions, weighted to the topics missed most, as a PDF to print.
    func printNew(count: Int) -> Data {
        let form = makeForm(
            bank: bank, number: ledger.nextFormNumber, count: count,
            missRates: missRates(attempts: ledger.attempts, bank: bank))
        let rendered = LearnerPDF.render(form: form, bank: bank)
        ledger.add(form, circles: rendered.circles)
        save()
        return rendered.data
    }

    /// Scores `marks` (each question's filled circles) for a printed page; only what the ledger keeps
    /// is saved, so a sample leaves nothing behind.
    func record(form number: Int, page: Int, marks: [[Int]], now: Date = Date()) -> Attempt? {
        let before = ledger
        let attempt = ledger.record(
            form: number, page: page, marks: marks, id: UUID().uuidString,
            date: Int(now.timeIntervalSince1970))
        if ledger != before { save() }
        return attempt
    }

    private func load<T: Decodable>(_ name: String) -> T? {
        guard let data = try? Data(contentsOf: folder.appendingPathComponent(name)) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    private func save() {
        let e = JSONEncoder()
        try? e.encode(ledger.forms).write(to: folder.appendingPathComponent("forms.json"), options: .atomic)
        try? e.encode(ledger.attempts).write(
            to: folder.appendingPathComponent("attempts.json"), options: .atomic)
        try? e.encode(ledger.circles).write(
            to: folder.appendingPathComponent("circles.json"), options: .atomic)
    }
}
