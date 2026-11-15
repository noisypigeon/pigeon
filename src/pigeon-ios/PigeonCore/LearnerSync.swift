// The learner's practice carried to their instance: the phone's files stay the cache, and an item
// is pending until a sync answer names it; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §5 and its 2026-11-15 amendment.

import Foundation

/// A form as a sync sends it: the form, with each page's circles as it printed.
public struct SyncForm: Codable, Equatable, Sendable {
    public let number: Int
    public let questionIDs: [Int]
    public let key: [Int]
    public let circles: [[PlacedCircle]]

    public init(number: Int, questionIDs: [Int], key: [Int], circles: [[PlacedCircle]]) {
        self.number = number
        self.questionIDs = questionIDs
        self.key = key
        self.circles = circles
    }
}

/// What `POST /v0/app/learner/sync` takes.
public struct SyncRequest: Codable, Equatable, Sendable {
    public let forms: [SyncForm]
    public let attempts: [Attempt]

    public init(forms: [SyncForm], attempts: [Attempt]) {
        self.forms = forms
        self.attempts = attempts
    }

    public var isEmpty: Bool { forms.isEmpty && attempts.isEmpty }
}

/// What it answers: every form number and attempt ID the instance holds, and what it refused.
public struct SyncAnswer: Codable, Equatable, Sendable {
    public let forms: [Int]
    public let attempts: [String]
    public let refused: [String]

    public init(forms: [Int], attempts: [String], refused: [String] = []) {
        self.forms = forms
        self.attempts = attempts
        self.refused = refused
    }
}

/// What the instance is known to hold, kept beside the ledger's files in `synced.json`.
public struct LearnerSynced: Codable, Equatable, Sendable {
    public var forms: Set<Int>
    public var attempts: Set<String>

    public init(forms: Set<Int> = [], attempts: Set<String> = []) {
        self.forms = forms
        self.attempts = attempts
    }
}

/// Sends a sync to the instance.
public protocol LearnerTransport: Sendable {
    func sync(_ request: SyncRequest) async throws -> SyncAnswer
}

/// The most items one request carries.
public let syncBatch = 400

/// Every form and attempt in `ledger` the instance isn't known to hold, forms first. The sample
/// is never in the ledger, so never sent.
public func pending(_ ledger: LearnerLedger, synced: LearnerSynced) -> SyncRequest {
    var seen = Set<Int>()
    var forms: [SyncForm] = []
    for f in ledger.forms.reversed() where !synced.forms.contains(f.number) && seen.insert(f.number).inserted {
        var pages: [[PlacedCircle]] = []
        var page = 0
        while let list = ledger.circles[LearnerLedger.pageKey(form: f.number, page: page)] {
            pages.append(list)
            page += 1
        }
        forms.append(SyncForm(number: f.number, questionIDs: f.questionIDs, key: f.key, circles: pages))
    }
    let attempts = ledger.attempts.filter { !synced.attempts.contains($0.id) }
    return SyncRequest(forms: forms.reversed(), attempts: attempts)
}

/// `request` cut into requests of at most `size` items, each attempt after its form.
public func batches(_ request: SyncRequest, size: Int = syncBatch) -> [SyncRequest] {
    var out: [SyncRequest] = []
    var i = 0
    while i < request.forms.count {
        let end = min(i + size, request.forms.count)
        out.append(SyncRequest(forms: Array(request.forms[i..<end]), attempts: []))
        i = end
    }
    i = 0
    while i < request.attempts.count {
        let end = min(i + size, request.attempts.count)
        out.append(SyncRequest(forms: [], attempts: Array(request.attempts[i..<end])))
        i = end
    }
    return out
}

/// What a sync left: what the instance is known to hold, what it refused, and the error that
/// stopped it, if one did. Nothing on the phone is ever removed.
public struct SyncOutcome: Sendable {
    public let synced: LearnerSynced
    public let refused: [String]
    public let error: Error?
}

/// Sends everything pending in `ledger`, batch by batch. Each answer names everything the
/// instance holds, so `synced` becomes exactly that; an item it doesn't name stays pending.
public func sync(
    _ ledger: LearnerLedger, synced: LearnerSynced, transport: LearnerTransport
) async -> SyncOutcome {
    var known = synced
    var refused: [String] = []
    for batch in batches(pending(ledger, synced: known)) {
        do {
            let answer = try await transport.sync(batch)
            known = LearnerSynced(forms: Set(answer.forms), attempts: Set(answer.attempts))
            refused += answer.refused
        } catch {
            return SyncOutcome(synced: known, refused: refused, error: error)
        }
    }
    return SyncOutcome(synced: known, refused: refused, error: nil)
}

/// Whether the instance holds everything the phone does.
public func carried(_ ledger: LearnerLedger, synced: LearnerSynced) -> Bool {
    pending(ledger, synced: synced).isEmpty
}

/// The ledger's files in a folder: `forms.json`, `attempts.json` and `circles.json`, as step 2
/// wrote them, and `synced.json` beside them.
public struct LearnerFiles: Sendable {
    public let folder: URL

    public init(folder: URL) { self.folder = folder }

    private func read<T: Decodable>(_ name: String) -> T? {
        guard let data = try? Data(contentsOf: folder.appendingPathComponent(name)) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    private func write<T: Encodable>(_ value: T, _ name: String) {
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try? JSONEncoder().encode(value).write(to: folder.appendingPathComponent(name), options: .atomic)
    }

    public func ledger() -> LearnerLedger {
        LearnerLedger(
            forms: read("forms.json") ?? [], attempts: read("attempts.json") ?? [],
            circles: read("circles.json") ?? [:])
    }

    public func save(_ ledger: LearnerLedger) {
        write(ledger.forms, "forms.json")
        write(ledger.attempts, "attempts.json")
        write(ledger.circles, "circles.json")
    }

    public func synced() -> LearnerSynced { read("synced.json") ?? LearnerSynced() }

    public func save(_ synced: LearnerSynced) { write(synced, "synced.json") }
}
