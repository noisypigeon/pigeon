// The carry-over: practice kept on the phone before the link reaches the instance whole, and
// nothing on the phone goes before the instance confirms it holds it.

import Foundation
import Testing

import PigeonCore

/// The instance's side of a sync, as the Rust store answers it: it keeps each form by number and
/// each attempt by ID, and names everything it holds.
private actor FakeInstance: LearnerTransport {
    var forms: [Int: SyncForm] = [:]
    var attempts: [String: Attempt] = [:]
    var requests = 0
    /// Fail the request with this number (1-based).
    let failOn: Int?
    /// Answer without these attempts, as an instance that lost or refused them.
    var forget: Set<String>

    init(failOn: Int? = nil, forget: Set<String> = []) {
        self.failOn = failOn
        self.forget = forget
    }

    func forgetNothing() { forget = [] }

    struct Down: Error {}

    func sync(_ request: SyncRequest) async throws -> SyncAnswer {
        requests += 1
        if requests == failOn { throw Down() }
        for f in request.forms { forms[f.number] = f }
        for a in request.attempts where forms[a.form] != nil && !forget.contains(a.id) {
            attempts[a.id] = a
        }
        return SyncAnswer(forms: Array(forms.keys), attempts: Array(attempts.keys))
    }
}

/// A folder holding step 2's three files, written as build 14 wrote them: three forms (one of two
/// pages), their circles, and five scored pages.
private func seededFolder() throws -> URL {
    let folder = FileManager.default.temporaryDirectory
        .appendingPathComponent("learner-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    var ledger = LearnerLedger()
    for (number, count) in [(17, 25), (18, 25), (19, 30)] {
        let ids = (0..<count).map { number * 100 + $0 }
        let form = Form(number: number, questionIDs: ids, key: ids.map { $0 % 4 })
        let pages = (0..<((count + 24) / 25)).map { page in
            (0..<4).map { PlacedCircle(question: page * 25, choice: $0, x: 20 + Double($0) * 9.5, y: 61.25) }
        }
        ledger.add(form, circles: pages)
    }
    var at = 1_794_700_000
    for (form, page) in [(17, 0), (17, 0), (18, 0), (19, 0), (19, 1)] {
        at += 3600
        let n = form == 19 && page == 1 ? 5 : 25
        _ = ledger.record(
            form: form, page: page, marks: (0..<n).map { [$0 % 4] }, id: UUID().uuidString, date: at)
    }
    let e = JSONEncoder()
    try e.encode(ledger.forms).write(to: folder.appendingPathComponent("forms.json"))
    try e.encode(ledger.attempts).write(to: folder.appendingPathComponent("attempts.json"))
    try e.encode(ledger.circles).write(to: folder.appendingPathComponent("circles.json"))
    return folder
}

private func bytes(_ folder: URL) throws -> [String: Data] {
    var out: [String: Data] = [:]
    for name in ["forms.json", "attempts.json", "circles.json"] {
        out[name] = try Data(contentsOf: folder.appendingPathComponent(name))
    }
    return out
}

@Suite struct LearnerSyncTests {
    @Test func everythingOnThePhoneBeforeTheLinkReachesTheInstanceWhole() async throws {
        let folder = try seededFolder()
        let before = try bytes(folder)
        let files = LearnerFiles(folder: folder)
        let ledger = files.ledger()
        #expect(ledger.forms.count == 3)
        #expect(ledger.attempts.count == 5)
        let instance = FakeInstance()
        let outcome = await sync(ledger, synced: files.synced(), transport: instance)
        #expect(outcome.error == nil)
        files.save(outcome.synced)
        #expect(carried(ledger, synced: files.synced()))
        let held = await instance.forms
        for f in ledger.forms {
            let sent = try #require(held[f.number])
            #expect(sent.questionIDs == f.questionIDs)
            #expect(sent.key == f.key)
            let pages = (0..<sent.circles.count).compactMap {
                ledger.circles[LearnerLedger.pageKey(form: f.number, page: $0)]
            }
            #expect(sent.circles == pages)
            #expect(!sent.circles.isEmpty)
        }
        #expect(held[19]?.circles.count == 2)
        let attempts = await instance.attempts
        for a in ledger.attempts { #expect(attempts[a.id] == a) }
        #expect(try bytes(folder) == before)
        #expect(files.ledger() == ledger)
    }

    @Test func aFailedSyncLosesNothingAndTheNextOneFinishes() async throws {
        let folder = try seededFolder()
        let before = try bytes(folder)
        let files = LearnerFiles(folder: folder)
        let ledger = files.ledger()
        let instance = FakeInstance(failOn: 1)
        let failed = await sync(ledger, synced: files.synced(), transport: instance)
        #expect(failed.error != nil)
        files.save(failed.synced)
        #expect(pending(ledger, synced: files.synced()) == pending(ledger, synced: LearnerSynced()))
        #expect(try bytes(folder) == before)
        let again = await sync(ledger, synced: files.synced(), transport: instance)
        #expect(again.error == nil)
        #expect(carried(ledger, synced: again.synced))
    }

    @Test func whatTheInstanceDoesntNameStaysPending() async throws {
        let folder = try seededFolder()
        let files = LearnerFiles(folder: folder)
        let ledger = files.ledger()
        let lost = ledger.attempts[3].id
        let partial = FakeInstance(forget: [lost])
        let outcome = await sync(ledger, synced: files.synced(), transport: partial)
        #expect(outcome.error == nil)
        let left = pending(ledger, synced: outcome.synced)
        #expect(left.forms.isEmpty)
        #expect(left.attempts.map(\.id) == [lost])
        #expect(files.ledger() == ledger)
        await partial.forgetNothing()
        let next = await sync(ledger, synced: outcome.synced, transport: partial)
        #expect(await partial.attempts.keys.contains(lost))
        #expect(carried(ledger, synced: next.synced))
    }

    @Test func aLargeLedgerGoesInBatchesWithEachFormBeforeItsAttempts() {
        var ledger = LearnerLedger()
        for n in 0..<3 {
            ledger.add(Form(number: 100 + n, questionIDs: [1], key: [0]), circles: [[]])
        }
        for i in 0..<10 {
            _ = ledger.record(form: 100 + i % 3, page: 0, marks: [[0]], id: "a\(i)", date: i)
        }
        let parts = batches(pending(ledger, synced: LearnerSynced()), size: 4)
        #expect(parts.map { $0.forms.count + $0.attempts.count } == [3, 4, 4, 2])
        #expect(parts[0].attempts.isEmpty)
    }

    @Test func theSyncBodyIsWhatTheInstanceReads() throws {
        let form = SyncForm(
            number: 17, questionIDs: [4, 9], key: [2, 0],
            circles: [[PlacedCircle(question: 0, choice: 1, x: 10.5, y: 20)]])
        let data = try JSONEncoder().encode(SyncRequest(forms: [form], attempts: []))
        let json = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let f = try #require((json["forms"] as? [[String: Any]])?.first)
        #expect(Set(f.keys) == ["number", "questionIDs", "key", "circles"])
        let c = try #require((f["circles"] as? [[[String: Any]]])?.first?.first)
        #expect(Set(c.keys) == ["question", "choice", "x", "y"])
    }
}
