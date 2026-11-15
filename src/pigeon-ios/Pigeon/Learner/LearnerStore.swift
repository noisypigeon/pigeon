// The learner's practice on this iPhone: the bank, and PigeonCore's ledger of printed forms, each page's
// circles and scored pages, kept as the cache of what the learner's instance holds once linked; the sample
// is never saved; see .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md step 2, its
// sample amendment and its 2026-11-15 standalone amendment.

import Foundation
import PigeonCore

/// The learner's instance, once this iPhone has joined it.
struct LearnerLink: Codable, Equatable {
    let base: URL

    private static let key = "learner.instance"

    static func load() -> LearnerLink? {
        UserDefaults.standard.data(forKey: key).flatMap { try? JSONDecoder().decode(LearnerLink.self, from: $0) }
    }

    func save() {
        UserDefaults.standard.set(try? JSONEncoder().encode(self), forKey: Self.key)
    }
}

/// Syncs through the instance's `POST /v0/app/learner/sync`, signed by this iPhone's key.
struct InstanceTransport: LearnerTransport {
    let base: URL

    func sync(_ request: SyncRequest) async throws -> SyncAnswer {
        let body = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]
        let api = API(device: try DeviceKey.load(), base: base)
        return try await api.call("POST", "/v0/app/learner/sync", body: body, as: SyncAnswer.self)
    }
}

@MainActor
final class LearnerStore: ObservableObject {
    @Published private(set) var ledger = LearnerLedger()
    /// What the instance is known to hold.
    @Published private(set) var synced = LearnerSynced()
    /// Why the last sync stopped, until one finishes.
    @Published private(set) var syncError: String?
    let bank: [Question]

    private let files: LearnerFiles
    private var syncing = false

    init() {
        let url = Bundle.main.url(forResource: "questions", withExtension: "json")
        bank = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? decodeBank($0) } ?? []
        files = LearnerFiles(
            folder: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("learner", isDirectory: true))
        ledger = files.ledger()
        synced = files.synced()
        Task { await sync() }
    }

    var forms: [Form] { ledger.forms }
    var attempts: [Attempt] { ledger.attempts }

    /// The best score so far, or nil before the first.
    var best: Int? { ledger.best }

    /// Whether this iPhone has joined the learner's instance.
    var linked: Bool { LearnerLink.load() != nil }

    /// Whether the instance holds everything kept here.
    var carried: Bool { PigeonCore.carried(ledger, synced: synced) }

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
        files.save(ledger)
        Task { await sync() }
        return rendered.data
    }

    /// Scores `marks` (each question's filled circles) for a printed page; only what the ledger keeps
    /// is saved, so a sample leaves nothing behind.
    func record(form number: Int, page: Int, marks: [[Int]], now: Date = Date()) -> Attempt? {
        let before = ledger
        let attempt = ledger.record(
            form: number, page: page, marks: marks, id: UUID().uuidString,
            date: Int(now.timeIntervalSince1970))
        if ledger != before {
            files.save(ledger)
            Task { await sync() }
        }
        return attempt
    }

    /// Sends what the instance doesn't hold yet. Nothing here is removed: an item stays pending
    /// until an answer names it, so a failed sync is retried on the next open or write.
    func sync() async {
        guard let link = LearnerLink.load(), !syncing else { return }
        syncing = true
        defer { syncing = false }
        let outcome = await PigeonCore.sync(ledger, synced: synced, transport: InstanceTransport(base: link.base))
        synced = outcome.synced
        files.save(outcome.synced)
        syncError = outcome.error?.localizedDescription ?? outcome.refused.first
    }
}
