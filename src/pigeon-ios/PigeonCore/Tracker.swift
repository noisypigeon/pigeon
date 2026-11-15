// The tracker: each session and its changes on a ten-stop rail, as /v0/app/tracker answers; see .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md's 2026-11-15 amendment.

import Foundation

/// A rail's mark for a change.
public enum RailMark: String, Codable, Equatable, Sendable {
    case ok, failed, stalled, done, held
}

/// The brief a change carries out.
public struct BriefRef: Codable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let adr: String
    public let step: String

    public init(id: String, title: String, adr: String, step: String) {
        self.id = id
        self.title = title
        self.adr = adr
        self.step = step
    }
}

/// One change on the rail.
public struct TrackedChange: Codable, Equatable, Identifiable, Sendable {
    public var id: Int { number }
    public let number: Int
    public let title: String
    public let stage: String
    public let needs: String
    public let waitsOn: Int?
    public let next: String
    public let since: Int?
    public let stop: Int
    public let mark: RailMark
    public let brief: BriefRef?

    public init(
        number: Int, title: String, stage: String, needs: String, waitsOn: Int? = nil,
        next: String, since: Int? = nil, stop: Int, mark: RailMark, brief: BriefRef? = nil
    ) {
        self.number = number
        self.title = title
        self.stage = stage
        self.needs = needs
        self.waitsOn = waitsOn
        self.next = next
        self.since = since
        self.stop = stop
        self.mark = mark
        self.brief = brief
    }

    enum CodingKeys: String, CodingKey {
        case number, title, stage, needs, next, since, stop, mark, brief
        case waitsOn = "waits_on"
    }
}

/// One session and its changes.
public struct TrackedSession: Codable, Equatable, Identifiable, Sendable {
    public var id: String { label }
    public let label: String
    public let model: String?
    public let role: String?
    public let planner: String?
    public let brief: String?
    public let listening: Bool
    public let changes: [TrackedChange]

    public init(
        label: String, model: String? = nil, role: String? = nil, planner: String? = nil,
        brief: String? = nil, listening: Bool, changes: [TrackedChange]
    ) {
        self.label = label
        self.model = model
        self.role = role
        self.planner = planner
        self.brief = brief
        self.listening = listening
        self.changes = changes
    }
}

/// How many changes wait on the owner, stall, or wait on a provider.
public struct TrackerCounts: Codable, Equatable, Sendable {
    public let owner: Int
    public let stalled: Int
    public let provider: Int

    public init(owner: Int, stalled: Int, provider: Int) {
        self.owner = owner
        self.stalled = stalled
        self.provider = provider
    }
}

/// /v0/app/tracker's answer.
public struct Tracker: Codable, Equatable, Sendable {
    public let sessions: [TrackedSession]
    public let counts: TrackerCounts

    public init(sessions: [TrackedSession], counts: TrackerCounts) {
        self.sessions = sessions
        self.counts = counts
    }
}

/// One event of the instance's log.
public struct AppEvent: Codable, Equatable, Identifiable, Sendable {
    public let id: Int
    public let at: Int
    public let kind: String
    public let change: Int?
    public let summary: String

    public init(id: Int, at: Int, kind: String, change: Int? = nil, summary: String) {
        self.id = id
        self.at = at
        self.kind = kind
        self.change = change
        self.summary = summary
    }
}

/// /v0/app/events' answer.
public struct AppEvents: Codable, Equatable, Sendable {
    public let last: Int
    public let events: [AppEvent]

    public init(last: Int, events: [AppEvent]) {
        self.last = last
        self.events = events
    }
}

/// The rail's ten stops, in order.
public let railStops = [
    "proposed", "authorized", "pushed", "checking", "checked", "approved", "queued", "merging",
    "merged", "deploying",
]

/// A session's label as the rail shows it: "…" and the last four characters of its last segment.
public func shortLabel(_ label: String) -> String {
    let segment = label.split(separator: "/").last.map(String.init) ?? label
    return "…" + String(segment.suffix(4))
}

/// The session's class as a badge.
public func modelBadge(_ model: String?) -> String {
    model?.uppercased() ?? "SESSION"
}

/// Who acts next on a change, and what.
public func needsChip(_ change: TrackedChange) -> String {
    switch change.needs {
    case "owner": return "needs you · " + change.next
    case "agent": return "agent · " + change.next
    case "provider": return "provider · " + change.next
    case "platform": return "platform"
    case "change": return change.waitsOn.map { "after !\($0)" } ?? "after another change"
    default: return ""
    }
}

/// Time since `since`, in `remaining`'s format.
public func elapsed(since: Int, now: Int) -> String {
    let seconds = max(0, now - since)
    if seconds < 60 { return "now" }
    if seconds < 3_600 { return "\(seconds / 60) min" }
    return "\(seconds / 3_600)h \(String(format: "%02d", (seconds % 3_600) / 60))m"
}

/// The tracker's filters.
public enum TrackerFilter: String, CaseIterable, Sendable {
    case all, owner, stalled, provider
}

/// The sessions keeping only the changes `f` matches.
public func filtered(_ t: Tracker, by f: TrackerFilter) -> [TrackedSession] {
    func keep(_ c: TrackedChange) -> Bool {
        switch f {
        case .all: return true
        case .owner: return c.needs == "owner"
        case .stalled: return c.mark == .stalled
        case .provider: return c.needs == "provider"
        }
    }
    if f == .all { return t.sessions }
    return t.sessions.compactMap { s in
        let kept = s.changes.filter(keep)
        guard !kept.isEmpty else { return nil }
        return TrackedSession(
            label: s.label, model: s.model, role: s.role, planner: s.planner, brief: s.brief,
            listening: s.listening, changes: kept)
    }
}

/// A change's rail for VoiceOver.
public func railLabel(_ c: TrackedChange) -> String {
    let base = "Stage \(c.stop + 1) of 10: \(c.stage)"
    switch c.mark {
    case .stalled: return base + ", stalled"
    case .failed: return base + ", failed"
    case .done: return base + ", done"
    case .held: return base + ", held"
    case .ok: return base
    }
}
