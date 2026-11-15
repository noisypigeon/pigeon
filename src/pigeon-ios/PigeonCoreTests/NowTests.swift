// PigeonCore: the Now tab's cards, queue and Needs me come out as the approved mockup shows; see
// .claude/decisions/2026-11-15-runtime-ios-app-now-tab.md.

import Foundation
import Testing

import PigeonCore

@Suite struct NowTests {
    static func change(
        _ n: Int, stage: String = "checked", needs: String = "none", mark: RailMark = .ok,
        waitsOn: Int? = nil, next: String = "go", since: Int? = nil
    ) -> TrackedChange {
        TrackedChange(
            number: n, title: "feat(ios): change \(n)", stage: stage, needs: needs, waitsOn: waitsOn,
            next: next, since: since, stop: 4, mark: mark)
    }

    static func session(_ label: String, _ model: String?, _ changes: [TrackedChange]) -> TrackedSession {
        TrackedSession(label: label, model: model, listening: true, changes: changes)
    }

    static func tracker(_ sessions: [TrackedSession], queue: [QueuedChange] = []) -> Tracker {
        Tracker(sessions: sessions, counts: TrackerCounts(owner: 0, stalled: 0, provider: 0), queue: queue)
    }

    @Test func aClassIsReadFromANameOrIDAndUnknownIsOther() {
        #expect(ModelClass("opus") == .opus)
        #expect(ModelClass("claude-fable-5-2") == .fable)
        #expect(ModelClass("Sonnet 5.5") == .sonnet)
        #expect(ModelClass("haiku") == .haiku)
        #expect(ModelClass("gpt") == .other)
        #expect(ModelClass(nil) == .other)
    }

    @Test func theTallyLeavesOutAClassAtZero() {
        let t = Self.tracker([
            Self.session("a", "opus", []), Self.session("b", "opus", []),
            Self.session("c", "sonnet", []), Self.session("d", "fable", []),
        ])
        let tally = classTally(t.sessions)
        #expect(tally.map { $0.model } == [.opus, .sonnet, .fable])
        #expect(tally.map { $0.count } == [2, 1, 1])
    }

    @Test func aTrackerWithoutAQueueDecodesAndTheQueueComesFromTheRows() throws {
        let json = """
            {"sessions": [{"label": "a", "model": "opus", "listening": true, "changes": [
              {"number": 568, "title": "t", "stage": "queued 3/14", "needs": "change",
               "waits_on": 578, "next": "!578 merges", "stop": 6, "mark": "ok"},
              {"number": 578, "title": "t", "stage": "merging", "needs": "platform",
               "next": "the merge job squashes it onto main", "stop": 7, "mark": "ok"},
              {"number": 579, "title": "t", "stage": "queued 8/14", "needs": "change",
               "waits_on": 578, "next": "!578 merges", "stop": 6, "mark": "ok"}]}],
             "counts": {"owner": 0, "stalled": 0, "provider": 0}}
            """
        let t = try JSONDecoder.pigeon.decode(Tracker.self, from: Data(json.utf8))
        #expect(t.queue.isEmpty)
        let q = mergeQueue(t)
        #expect(q.map(\.number) == [578, 568, 579])
        #expect(q.map(\.position) == [0, 3, 8])
        #expect(q[1].model == "opus")
        #expect(queueWait(q[1]) == "after !578")
    }

    @Test func theControlPlanesQueueWinsAndKeepsASessionlessChange() throws {
        let json = """
            {"sessions": [], "counts": {"owner": 0, "stalled": 0, "provider": 0},
             "queue": [
              {"number": 547, "title": "t", "session": "s", "model": "opus", "stage": "queued 2/14",
               "position": 2, "needs": "change", "waits_on": 546, "next": "!546 merges or closes; it keeps its place"},
              {"number": 600, "title": "t", "stage": "queued 11/14", "position": 11,
               "needs": "change", "next": "go"},
              {"number": 578, "title": "t", "session": "s", "stage": "merging", "position": 0,
               "needs": "platform", "next": "squash"}]}
            """
        let t = try JSONDecoder.pigeon.decode(Tracker.self, from: Data(json.utf8))
        let q = mergeQueue(t)
        #expect(q.map(\.number) == [578, 547, 600])
        #expect(q[2].session == nil)
        #expect(queueWait(q[1]) == "after !546 · keeps its place")
        #expect(queueWait(q[0]) == "squash")
    }

    @Test func positionsAreReadFromTheStage() {
        #expect(queuePosition("merging") == 0)
        #expect(queuePosition("queued 9/14") == 9)
        #expect(queuePosition("approved") == nil)
    }

    @Test func needsMeCountsAChangeAndItsApprovalOnce() throws {
        let home = try JSONDecoder.pigeon.decode(
            Home.self,
            from: Data(
                """
                {"insights": {"cost": null, "errors": null},
                 "approvals": [{"id": "k1", "number": 584, "operation": "change.merge", "title": "t", "expires": 9}],
                 "auto": null,
                 "items": [{"id": 1, "kind": "decision", "session": "s", "change": null, "title": "cap?",
                   "detail": "d", "options": ["a", "b"], "opened": 5}]}
                """.utf8))
        let t = Self.tracker([
            Self.session(
                "s", "opus",
                [
                    Self.change(584, needs: "owner"), Self.change(592, needs: "owner", next: "allow the rerun"),
                    Self.change(9),
                ])
        ])
        let items = needsMe(home: home, tracker: t)
        #expect(items.map(\.id) == ["item-1", "approval-k1", "change-592"])
        #expect(needsSummary(items) == "1 approval · 1 decision · 1 change at needs you")
        #expect(needsSummary([]) == "Nothing waiting")
        #expect(needsMe(home: nil, tracker: nil).isEmpty)
    }

    @Test func aSessionsLineSaysWhatItDoesOrNeeds() {
        #expect(doingLine(Self.session("s", "opus", [Self.change(1, needs: "owner", next: "approve")])) == "Needs you: approve")
        #expect(doingLine(Self.session("s", "opus", [Self.change(2, stage: "merging")])) == "Merging !2 now")
        #expect(
            doingLine(Self.session("s", "opus", [Self.change(3, stage: "failed", needs: "agent", mark: .failed, next: "fix")]))
                == "Fixing: fix")
        #expect(doingLine(Self.session("s", "opus", [Self.change(4, stage: "checking 7/8")])) == "Waiting on checks: 7 of 8 done")
        #expect(doingLine(Self.session("s", "opus", [Self.change(5, stage: "queued 9/14")])) == "Queued at #9")
        #expect(doingLine(Self.session("s", "opus", [])) == "No open change")
        #expect(sessionTitle(Self.session("s", "opus", [Self.change(5), Self.change(7)])) == "change 7")
        #expect(sessionTitle(Self.session("s", "fable", [])) == "Fable session")
    }

    @Test func aChangesSheetDecodesFromChangeStatus() throws {
        let json = """
            {"change": {"number": 586, "title": "fix(ios): t", "state": "approved", "session": "s",
               "jobs": [{"name": "ios", "argv": [], "status": "passed", "attempts": 1}]},
             "stage": {"stage": "queued 9/14", "needs": "change", "waits_on": 578,
               "next": "!578 merges", "since": 5},
             "github": [{"checks": ["ios"], "run": "https://github.com/x", "sha": "c", "waiting": null}],
             "branch": "change/586-x", "failures": {}, "position": 9, "model": "opus", "approval": null}
            """
        let d = try JSONDecoder.pigeon.decode(ChangeDetail.self, from: Data(json.utf8))
        #expect(d.number == 586)
        #expect(d.position == 9)
        #expect(d.waitsOn == 578)
        #expect(d.model == "opus")
        #expect(d.jobs.first?.status == "passed")
        #expect(d.github.first?.run == "https://github.com/x")
        let bare = try JSONDecoder.pigeon.decode(
            ChangeDetail.self,
            from: Data(
                #"{"change": {"number": 1, "title": "t"}, "stage": {"stage": "merged", "needs": "none", "next": "nothing"}, "github": {}}"#
                    .utf8))
        #expect(bare.jobs.isEmpty)
        #expect(bare.github.isEmpty)
        #expect(bare.position == nil)
    }
}
