// The control plane's answers to the app, as /v0/app/* returns them; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §4.

import Foundation

/// One part of a stacked bar: a source or a kind, and its value.
public struct Part: Codable, Equatable, Sendable {
    public let name: String
    public let value: Double

    public init(name: String, value: Double) {
        self.name = name
        self.value = value
    }
}

/// One bar: an hour or a day, from `at`.
public struct Bucket: Codable, Equatable, Sendable {
    public let at: Int
    public let parts: [Part]

    public init(at: Int, parts: [Part]) {
        self.at = at
        self.parts = parts
    }

    /// The bar's height in the metric's unit.
    public var total: Double { parts.reduce(0) { $0 + $1.value } }
}

/// A legend entry: a source or kind, and its total over the range.
public struct LegendEntry: Codable, Equatable, Sendable {
    public let name: String
    public let total: Double
}

/// A chart of `metric` over `range` days.
public struct Stacks: Codable, Equatable, Sendable {
    public let metric: String
    public let source: String?
    public let range: Int
    public let step: Int
    public let total: Double
    public let estimate: Bool
    public let legend: [LegendEntry]
    public let buckets: [Bucket]

    public init(
        metric: String, source: String?, range: Int, step: Int, total: Double, estimate: Bool,
        legend: [LegendEntry], buckets: [Bucket]
    ) {
        self.metric = metric
        self.source = source
        self.range = range
        self.step = step
        self.total = total
        self.estimate = estimate
        self.legend = legend
        self.buckets = buckets
    }
}

/// A waiting approval, as the list shows it.
public struct ApprovalSummary: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let number: Int
    public let operation: String
    public let title: String
    public let expires: Int
}

/// A check on the change's head.
public struct CheckRun: Codable, Equatable, Sendable {
    public let name: String
    public let status: String
}

/// One approval's card.
public struct ApprovalDetail: Codable, Equatable, Sendable {
    public let id: String?
    public let number: Int
    public let operation: String
    public let title: String
    public let branch: String
    public let head: String?
    public let state: String
    public let scope: [String]
    public let plan: String
    public let checks: [CheckRun]
    public let tier: String?
    public let rule: String?
    public let expires: Int

    enum CodingKeys: String, CodingKey {
        case id, number, operation, title, branch, head, state, scope, plan, checks, tier, rule,
            expires
    }
}

/// An Auto window, running or paused.
public struct AutoWindow: Codable, Equatable, Sendable {
    public let id: Int
    public let hours: Int
    public let started: Int
    public let expires: Int
    public let used: Int
    public let state: String
    public let why: String?

    public init(
        id: Int, hours: Int, started: Int, expires: Int, used: Int, state: String, why: String?
    ) {
        self.id = id
        self.hours = hours
        self.started = started
        self.expires = expires
        self.used = used
        self.state = state
        self.why = why
    }
}

/// `GET /v0/app/home`.
public struct Home: Codable, Equatable, Sendable {
    public struct Insights: Codable, Equatable, Sendable {
        public let cost: Stacks?
        public let errors: Stacks?
    }

    public let insights: Insights
    public let approvals: [ApprovalSummary]
    public let auto: AutoWindow?
}

/// `POST /v0/app/challenge`.
public struct PasskeyChallenge: Codable, Equatable, Sendable {
    public let challenge: String
    public let ids: [String]
    public let rp: String
}

extension JSONDecoder {
    /// A decoder for the control plane's JSON.
    public static var pigeon: JSONDecoder { JSONDecoder() }
}
