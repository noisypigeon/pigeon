// Chart geometry and the words around it, kept out of the views so they can be tested.

import Foundation

/// One drawn segment of a bar: which part, how tall, and where it sits from the baseline.
public struct Segment: Equatable, Sendable {
    public let name: String
    public let value: Double
    public let height: Double
    public let offset: Double
}

/// The segments of `bucket`, bottom first, scaled so the tallest bar in `buckets` is `height`.
/// A part of 0 draws nothing; a part above 0 is at least `minimum` tall.
public func segments(
    of bucket: Bucket, among buckets: [Bucket], height: Double, minimum: Double = 2
) -> [Segment] {
    let tallest = buckets.map(\.total).max() ?? 0
    guard tallest > 0 else { return [] }
    var offset = 0.0
    var out: [Segment] = []
    for part in bucket.parts where part.value > 0 {
        let h = max(minimum, part.value / tallest * height)
        out.append(Segment(name: part.name, value: part.value, height: h, offset: offset))
        offset += h
    }
    return out
}

/// A metric's value in words: dollars to the cent for cost, a whole count for errors.
public func formatted(_ value: Double, metric: String) -> String {
    if metric == "cost" {
        return String(format: "$%.2f", value)
    }
    return String(Int(value.rounded()))
}

/// When a bucket starts, as the chart labels it: the hour for a day's chart, else how many
/// days ago, both in UTC.
public func label(at: Int, step: Int, now: Int) -> String {
    if step < 86_400 {
        let hour = (at % 86_400) / 3_600
        return String(format: "%02d:00", hour)
    }
    let days = (now - now % 86_400 - (at - at % 86_400)) / 86_400
    switch days {
    case 0: return "Today"
    case 1: return "Yesterday"
    default: return "\(days) days ago"
    }
}

/// How long until `expires` from `now`, as the Auto banner says it.
public func remaining(until expires: Int, now: Int) -> String {
    let left = max(0, expires - now)
    let hours = left / 3_600
    let minutes = (left % 3_600) / 60
    return hours > 0 ? "\(hours)h \(String(format: "%02d", minutes))m" : "\(minutes)m"
}

/// The sources, in the app's order.
public let sources = ["cloudflare", "github", "buildkite"]

/// A source's name as the app shows it.
public func sourceName(_ source: String) -> String {
    switch source {
    case "cloudflare": return "Cloudflare"
    case "github": return "GitHub"
    case "buildkite": return "Buildkite"
    default: return source
    }
}

/// A tier as the card's badge shows it.
public func tierBadge(_ tier: String?) -> String {
    (tier ?? "code").uppercased()
}
