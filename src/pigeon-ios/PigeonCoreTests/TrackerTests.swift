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

@Suite struct GlanceableTrackerTests {
    static func change(
        _ n: Int, stage: String = "checked", needs: String = "none", mark: RailMark = .ok,
        waitsOn: Int? = nil
    ) -> TrackedChange {
        TrackedChange(
            number: n, title: "feat(ios): x", stage: stage, needs: needs, waitsOn: waitsOn,
            next: "n", stop: 4, mark: mark)
    }

    static func session(_ label: String, _ changes: [TrackedChange], listening: Bool = true)
        -> TrackedSession
    {
        TrackedSession(label: label, listening: listening, changes: changes)
    }

    @Test func aSessionIsSilentOnlyWhenNoChangeNeedsAnyone() {
        #expect(isSilent(Self.session("a", [])))
        #expect(isSilent(Self.session("b", [Self.change(1, stage: "done", mark: .done)])))
        #expect(isSilent(Self.session("c", [Self.change(1, stage: "merged")], listening: false)))
        #expect(!isSilent(Self.session("d", [Self.change(1, needs: "platform")])))
        #expect(!isSilent(Self.session("e", [Self.change(1, needs: "agent", mark: .stalled)])))
        #expect(!isSilent(Self.session("f", [Self.change(1, needs: "platform", mark: .held)])))
        #expect(
            !isSilent(
                Self.session(
                    "g", [Self.change(1, stage: "done", mark: .done), Self.change(2, needs: "owner")]
                )))
    }

    @Test func silentSessionsAreHiddenUnlessShownAndThenLast() {
        let t = Tracker(
            sessions: [
                Self.session("quiet", []),
                Self.session("busy", [Self.change(1, needs: "provider")]),
                Self.session("owed", [Self.change(2, needs: "owner")]),
            ],
            counts: TrackerCounts(owner: 1, stalled: 0, provider: 1))
        #expect(silentCount(t) == 1)
        #expect(visible(t, by: .all, showSilent: false).map(\.label) == ["busy", "owed"])
        #expect(visible(t, by: .all, showSilent: true).map(\.label) == ["busy", "owed", "quiet"])
        #expect(visible(t, by: .owner, showSilent: true).map(\.label) == ["owed"])
    }

    /// Tonight's queue (15 Nov, build 16): changes queued behind !570, !570 merging, !584 checking,
    /// and sessions with only merged changes or none.
    static let tonight = Tracker(
        sessions: [
            session("hostnames", [
                change(570, stage: "merging", needs: "platform"),
                change(571, stage: "queued 4/14", needs: "change", waitsOn: 570),
                change(580, stage: "queued 5/14", needs: "change", waitsOn: 570),
            ]),
            session("standing", [
                change(546, stage: "queued 13/14", needs: "change", waitsOn: 570),
                change(547, stage: "queued 2/14", needs: "change", waitsOn: 546),
            ]),
            session("learner", [
                change(568, stage: "queued 11/14", needs: "change", waitsOn: 570),
                change(579, stage: "proposed", needs: "platform"),
            ]),
            session("noisypigeon", [change(584, stage: "checking 1/2", needs: "provider")]),
            session("restacked", [change(590, stage: "restacking", needs: "platform")]),
            session("merged", [change(565, stage: "merged")]),
            session("ended", [change(549, stage: "merged")]),
            session("ticket-only", []),
            session("idle", [], listening: false),
        ],
        counts: TrackerCounts(owner: 0, stalled: 0, provider: 1))

    @Test func queuedRestackingCheckingAndMergingChangesKeepTheirSessionShown() {
        for (stage, needs) in [
            ("queued 3/14", "change"), ("restacking", "platform"), ("checking 1/2", "provider"),
            ("merging", "platform"), ("proposed", "platform"),
        ] {
            let s = Self.session("s", [Self.change(1, stage: stage, needs: needs)])
            #expect(!isSilent(s), "\(stage)")
        }
        let shown = visible(Self.tonight, by: .all, showSilent: false)
        #expect(
            shown.map(\.label) == ["hostnames", "standing", "learner", "noisypigeon", "restacked"])
        #expect(shown.reduce(0) { $0 + $1.changes.count } == 9)
        #expect(silentCount(Self.tonight) == 4)
    }

    @Test func aFilterThatMatchesNothingFallsBackToAll() {
        let t = Self.tonight
        #expect(effectiveFilter(t, .owner) == .all)
        #expect(effectiveFilter(t, .stalled) == .all)
        #expect(effectiveFilter(t, .provider) == .provider)
        #expect(
            visible(t, by: .stalled, showSilent: false) == visible(t, by: .all, showSilent: false))
        #expect(silentCount(t, by: .stalled) == 4)
        #expect(visible(t, by: .provider, showSilent: false).map(\.label) == ["noisypigeon"])
        #expect(silentCount(t, by: .provider) == 0)
    }

    @Test func aRowsPillSaysWhoActsAndWhy() {
        #expect(
            rowStatus(Self.change(1, needs: "owner")) == RowStatus(kind: .owner, word: "Needs you"))
        #expect(
            rowStatus(Self.change(1, stage: "failed", needs: "agent", mark: .failed))
                == RowStatus(kind: .failed, word: "Failed"))
        #expect(
            rowStatus(Self.change(1, needs: "agent", mark: .stalled))
                == RowStatus(kind: .stalled, word: "Stalled", note: "its agent is quiet"))
        #expect(
            rowStatus(Self.change(1, stage: "held", needs: "platform", mark: .held))
                == RowStatus(kind: .held, word: "Held", note: "until a deploy passes"))
        #expect(
            rowStatus(Self.change(1, stage: "done", mark: .done))
                == RowStatus(kind: .done, word: "Done"))
        #expect(
            rowStatus(Self.change(1, stage: "queued 3/4", needs: "change", waitsOn: 506))
                == RowStatus(kind: .moving, word: "Queued 3/4", note: "after !506"))
        #expect(
            rowStatus(Self.change(1, stage: "checking 7/8", needs: "provider"))
                == RowStatus(kind: .moving, word: "Checking 7/8"))
    }

    @Test func aTitleSplitsIntoItsTagAndSubject() {
        #expect(scopeAndSubject("feat(backplane): owner_ask flags").tag == "backplane")
        #expect(scopeAndSubject("feat(backplane): owner_ask flags").subject == "owner_ask flags")
        #expect(scopeAndSubject("fix!: breaks").tag == "fix")
        #expect(scopeAndSubject("ci(ios)!: x").tag == "ios")
        #expect(scopeAndSubject("Plain title: with colon").tag == nil)
        #expect(scopeAndSubject("Plain title: with colon").subject == "Plain title: with colon")
        #expect(scopeAndSubject("no colon").tag == nil)
    }

    @Test func onlyAClaudeLinkOpensASession() {
        #expect(sessionLink("https://claude.ai/code/session_X1")?.path == "/code/session_X1")
        #expect(sessionLink("http://claude.ai/code/x") == nil)
        #expect(sessionLink("https://example.com/code/x") == nil)
        #expect(sessionLink("bare") == nil)
    }
}
