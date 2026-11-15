// The learner's practice on this iPhone: the bank, printed forms with each page's circles, and scored
// pages, kept until the learner's instance holds them; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md step 2.

import Foundation
import PigeonCore

@MainActor
final class LearnerStore: ObservableObject {
    @Published private(set) var forms: [Form] = []
    @Published private(set) var attempts: [Attempt] = []
    @Published private(set) var circles: [String: [PlacedCircle]] = [:]
    let bank: [Question]

    private let folder: URL

    init() {
        let url = Bundle.main.url(forResource: "questions", withExtension: "json")
        bank = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? decodeBank($0) } ?? []
        folder = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("learner", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        forms = load("forms.json") ?? []
        attempts = load("attempts.json") ?? []
        circles = load("circles.json") ?? [:]
    }

    /// The key a printed page's circles are kept under.
    static func pageKey(form: Int, page: Int) -> String { "\(form)-\(page)" }

    /// The best score so far, or nil before the first.
    var best: Int? { attempts.map(\.correct).max() }

    /// The newest form with `number`.
    func form(_ number: Int) -> Form? { forms.last { $0.number == number } }

    /// A new form of `count` questions, weighted to the topics missed most, as a PDF to print.
    func printNew(count: Int) -> Data {
        let number = ((forms.map(\.number).max() ?? 16) + 1) % 1024
        let form = makeForm(
            bank: bank, number: number, count: count,
            missRates: missRates(attempts: attempts, bank: bank))
        let rendered = LearnerPDF.render(form: form, bank: bank)
        forms.append(form)
        for (page, list) in rendered.circles.enumerated() {
            circles[Self.pageKey(form: number, page: page)] = list
        }
        save()
        return rendered.data
    }

    /// Scores `marks` (each question's filled circles) for a printed page, and keeps the attempt.
    func record(form number: Int, page: Int, marks: [[Int]], now: Date = Date()) -> Attempt? {
        guard let f = form(number) else { return nil }
        let slice = pageSlice(f, page: page)
        let attempt = Attempt(
            id: UUID().uuidString, form: number, page: page, date: Int(now.timeIntervalSince1970),
            questionIDs: slice.ids, marks: marks, results: score(key: slice.key, marks: marks))
        attempts.append(attempt)
        save()
        return attempt
    }

    private func load<T: Decodable>(_ name: String) -> T? {
        guard let data = try? Data(contentsOf: folder.appendingPathComponent(name)) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    private func save() {
        let e = JSONEncoder()
        try? e.encode(forms).write(to: folder.appendingPathComponent("forms.json"), options: .atomic)
        try? e.encode(attempts).write(to: folder.appendingPathComponent("attempts.json"), options: .atomic)
        try? e.encode(circles).write(to: folder.appendingPathComponent("circles.json"), options: .atomic)
    }
}
