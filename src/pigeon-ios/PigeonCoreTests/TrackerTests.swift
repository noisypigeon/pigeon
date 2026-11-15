// PigeonCore: the tracker's and ops' answers decode, and the rail's words, filters and bars
// come out as the mockup shows.

import Foundation
import Testing

import PigeonCore

@Suite struct TrackerTests {
    static func change(
        _ n: Int, needs: String = "none", mark: RailMark = .ok, waitsOn: Int? = nil,
        next: String = "merge"
    ) -> TrackedChange {
        TrackedChange(
            number: n, title: "t", stage: "checked", needs: needs, waitsOn: waitsOn, next: next,
            stop: 4, mark: mark)
    }

    @Test func aTrackerAnswerDecodes() throws {
        let json = """
            {"sessions": [{"label": "https://claude.ai/code/session_X1", "model": "sonnet",
              "role": "executor", "planner": "p", "brief": "b15", "listening": true,
              "changes": [{"number": 482, "title": "feat: x", "stage": "checked",
                "needs": "owner", "waits_on": 3, "next": "approve", "since": 1800000000,
                "stop": 4, "mark": "stalled",
                "brief": {"id": "b15", "title": "T", "adr": "a.md", "step": "s"}}]},
             {"label": "bare", "listening": false, "changes": []}],
             "counts": {"owner": 1, "stalled": 1, "provider": 0}}
            """
        let t = try JSONDecoder.pigeon.decode(Tracker.self, from: Data(json.utf8))
        #expect(t.sessions.count == 2)
        let c = t.sessions[0].changes[0]
        #expect(t.sessions[0].id == "https://claude.ai/code/session_X1")
        #expect(t.sessions[0].model == "sonnet")
        #expect(c.id == 482)
        #expect(c.waitsOn == 3)
        #expect(c.since == 1_800_000_000)
        #expect(c.mark == .stalled)
        #expect(c.brief?.id == "b15")
        #expect(t.sessions[1].model == nil)
        #expect(t.counts == TrackerCounts(owner: 1, stalled: 1, provider: 0))
    }

    @Test func anOpsAnswerDecodes() throws {
        let json = """
            {"range": 7,
             "deploys": {"passed": 2, "total": 3,
               "recent": [{"sha": "abc", "outcome": "passed", "at": 5}]},
             "queue": {"depth": 1, "merging": null, "waits": [4, 9], "p50": 6, "max": null},
             "approvals": {"count": 2, "p50": 3, "p95": null},
             "throughput": {"days": [{"day": 86400, "merged": 2}], "lead_p50": 75,
               "by": {"owner": 1, "window": 2, "mandate": 3, "policy": 4}},
             "providers": [{"provider": "github", "health": "ok", "minutes": 12.5, "cap": null}],
             "jobs": [{"name": "ios", "change": null, "status": "failed", "at": 9}],
             "note": null}
            """
        let o = try JSONDecoder.pigeon.decode(Ops.self, from: Data(json.utf8))
        #expect(o.range == 7)
        #expect(o.deploys.recent[0].id == "abc")
        #expect(o.queue.merging == nil)
        #expect(o.queue.waits == [4, 9])
        #expect(o.approvals.p95 == nil)
        #expect(o.throughput.leadP50 == 75)
        #expect(o.throughput.by.policy == 4)
        #expect(o.providers[0].id == "github")
        #expect(o.providers[0].minutes == 12.5)
        #expect(o.providers[0].cap == nil)
        #expect(o.jobs[0].change == nil)
        #expect(o.note == nil)
    }

    @Test func anEventsAnswerDecodes() throws {
        let json = """
            {"last": 9, "other": true, "events": [
              {"id": 8, "at": 1, "kind": "pushed", "change": 482, "summary": "pushed"},
              {"id": 9, "at": 2, "kind": "deploy", "change": null, "summary": "deployed"}]}
            """
        let e = try JSONDecoder.pigeon.decode(AppEvents.self, from: Data(json.utf8))
        #expect(e.last == 9)
        #expect(e.events[0].change == 482)
        #expect(e.events[1].change == nil)
        #expect(e.events[1].id == 9)
    }

    @Test func shortLabelKeepsTheLastFour() {
        #expect(shortLabel("https://claude.ai/code/session_015XFA1kVXKchUMBvxbUE5dL") == "…E5dL")
        #expect(shortLabel("abc") == "…abc")
    }

    @Test func modelBadgeNamesTheClass() {
        #expect(modelBadge("sonnet") == "SONNET")
        #expect(modelBadge(nil) == "SESSION")
    }

    @Test func needsChipSaysWhoActs() {
        #expect(needsChip(Self.change(1, needs: "owner", next: "approve")) == "needs you · approve")
        #expect(needsChip(Self.change(1, needs: "agent", next: "push")) == "agent · push")
        #expect(needsChip(Self.change(1, needs: "provider", next: "wait")) == "provider · wait")
        #expect(needsChip(Self.change(1, needs: "platform")) == "platform")
        #expect(needsChip(Self.change(1, needs: "change", waitsOn: 7)) == "after !7")
        #expect(needsChip(Self.change(1, needs: "change")) == "after another change")
        #expect(needsChip(Self.change(1, needs: "none")) == "")
    }

    @Test func elapsedMatchesRemainingsFormat() {
        #expect(elapsed(since: 100, now: 159) == "now")
        #expect(elapsed(since: 100, now: 160) == "1 min")
        #expect(elapsed(since: 0, now: 3_599) == "59 min")
        #expect(elapsed(since: 0, now: 3_600) == "1h 00m")
        #expect(elapsed(since: 0, now: 3_600 + 5 * 60) == "1h 05m")
    }

    @Test func filtersKeepOnlyMatchingChanges() {
        let a = Self.change(1, needs: "owner")
        let b = Self.change(2, needs: "provider", mark: .stalled)
        let c = Self.change(3)
        let t = Tracker(
            sessions: [
                TrackedSession(label: "s1", listening: true, changes: [a, b]),
                TrackedSession(label: "s2", listening: false, changes: [c]),
            ],
            counts: TrackerCounts(owner: 1, stalled: 1, provider: 1))
        #expect(filtered(t, by: .all) == t.sessions)
        #expect(filtered(t, by: .owner).map(\.changes) == [[a]])
        #expect(filtered(t, by: .stalled).map(\.changes) == [[b]])
        #expect(filtered(t, by: .provider).map(\.label) == ["s1"])
        #expect(TrackerFilter.allCases.count == 4)
    }

    @Test func railLabelNamesTheStageAndMark() {
        #expect(railLabel(Self.change(1)) == "Stage 5 of 10: checked")
        #expect(railLabel(Self.change(1, mark: .stalled)) == "Stage 5 of 10: checked, stalled")
        #expect(railLabel(Self.change(1, mark: .failed)) == "Stage 5 of 10: checked, failed")
        #expect(railLabel(Self.change(1, mark: .done)) == "Stage 5 of 10: checked, done")
        #expect(railStops.count == 10)
    }

    @Test func aHeldMarkDecodesAndIsSpoken() throws {
        let json = """
            {"number": 484, "title": "t", "stage": "held", "needs": "owner", "next": "resume",
             "stop": 5, "mark": "held"}
            """
        let c = try JSONDecoder.pigeon.decode(TrackedChange.self, from: Data(json.utf8))
        #expect(c.mark == .held)
        #expect(railLabel(c) == "Stage 6 of 10: held, held")
    }

    @Test func minutesTextHandlesNilMinutesAndHours() {
        #expect(minutesText(nil) == "—")
        #expect(minutesText(7) == "7 min")
        #expect(minutesText(60) == "1h 00m")
        #expect(minutesText(125) == "2h 05m")
    }

    @Test func barHeightsScaleToTheMaximum() {
        #expect(barHeights([1, 2, 4]) == [0.25, 0.5, 1])
        #expect(barHeights([0, 0]) == [0, 0])
        #expect(barHeights([]) == [])
    }
}
