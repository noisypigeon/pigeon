// Operational insights, as /v0/app/ops answers; see the iOS ADR's 2026-11-15 amendment.

import Foundation

/// One deploy of `main`.
public struct OpsDeploy: Codable, Equatable, Identifiable, Sendable {
    public var id: String { sha }
    public let sha: String
    public let outcome: String
    public let at: Int

    public init(sha: String, outcome: String, at: Int) {
        self.sha = sha
        self.outcome = outcome
        self.at = at
    }
}

public struct OpsDeploys: Codable, Equatable, Sendable {
    public let passed: Int
    public let total: Int
    public let recent: [OpsDeploy]

    public init(passed: Int, total: Int, recent: [OpsDeploy]) {
        self.passed = passed
        self.total = total
        self.recent = recent
    }
}

public struct OpsQueue: Codable, Equatable, Sendable {
    public let depth: Int
    public let merging: Int?
    public let waits: [Int]
    public let p50: Int?
    public let max: Int?

    public init(depth: Int, merging: Int? = nil, waits: [Int], p50: Int? = nil, max: Int? = nil) {
        self.depth = depth
        self.merging = merging
        self.waits = waits
        self.p50 = p50
        self.max = max
    }
}

public struct OpsApprovals: Codable, Equatable, Sendable {
    public let count: Int
    public let p50: Int?
    public let p95: Int?

    public init(count: Int, p50: Int? = nil, p95: Int? = nil) {
        self.count = count
        self.p50 = p50
        self.p95 = p95
    }
}

/// One day's merges, from `day` (Unix seconds at UTC midnight).
public struct OpsDay: Codable, Equatable, Sendable {
    public let day: Int
    public let merged: Int

    public init(day: Int, merged: Int) {
        self.day = day
        self.merged = merged
    }
}

/// Who approved the merges.
public struct OpsApprovers: Codable, Equatable, Sendable {
    public let owner: Int
    public let window: Int
    public let mandate: Int
    public let policy: Int

    public init(owner: Int, window: Int, mandate: Int, policy: Int) {
        self.owner = owner
        self.window = window
        self.mandate = mandate
        self.policy = policy
    }
}

public struct OpsThroughput: Codable, Equatable, Sendable {
    public let days: [OpsDay]
    public let leadP50: Int?
    public let by: OpsApprovers

    public init(days: [OpsDay], leadP50: Int? = nil, by: OpsApprovers) {
        self.days = days
        self.leadP50 = leadP50
        self.by = by
    }

    enum CodingKeys: String, CodingKey {
        case days, by
        case leadP50 = "lead_p50"
    }
}

public struct OpsProvider: Codable, Equatable, Identifiable, Sendable {
    public var id: String { provider }
    public let provider: String
    public let health: String
    public let minutes: Double
    public let cap: Double?

    public init(provider: String, health: String, minutes: Double, cap: Double? = nil) {
        self.provider = provider
        self.health = health
        self.minutes = minutes
        self.cap = cap
    }
}

/// A job that did not finish well.
public struct OpsJob: Codable, Equatable, Sendable {
    public let name: String
    public let change: Int?
    public let status: String
    public let at: Int

    public init(name: String, change: Int? = nil, status: String, at: Int) {
        self.name = name
        self.change = change
        self.status = status
        self.at = at
    }
}

/// /v0/app/ops' answer.
public struct Ops: Codable, Equatable, Sendable {
    public let range: Int
    public let deploys: OpsDeploys
    public let queue: OpsQueue
    public let approvals: OpsApprovals
    public let throughput: OpsThroughput
    public let providers: [OpsProvider]
    public let jobs: [OpsJob]
    public let note: String?

    public init(
        range: Int, deploys: OpsDeploys, queue: OpsQueue, approvals: OpsApprovals,
        throughput: OpsThroughput, providers: [OpsProvider], jobs: [OpsJob], note: String? = nil
    ) {
        self.range = range
        self.deploys = deploys
        self.queue = queue
        self.approvals = approvals
        self.throughput = throughput
        self.providers = providers
        self.jobs = jobs
        self.note = note
    }
}

/// Minutes as text: "—" for none, "N min", or "Nh MMm".
public func minutesText(_ m: Int?) -> String {
    guard let m else { return "—" }
    if m < 60 { return "\(m) min" }
    return "\(m / 60)h \(String(format: "%02d", m % 60))m"
}

/// Each value over the maximum; 0 when the maximum is 0.
public func barHeights(_ values: [Int]) -> [Double] {
    let top = values.max() ?? 0
    return values.map { top == 0 ? 0 : Double($0) / Double(top) }
}
