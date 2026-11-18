// The view contract the server validates in pigeon-core/src/view.rs; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §1.

import Foundation

/// Any JSON value, for arguments and structured content.
public enum JSONValue: Codable, Equatable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case null
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let b = try? c.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? c.decode(Double.self) {
            self = .number(n)
        } else if let s = try? c.decode(String.self) {
            self = .string(s)
        } else if let a = try? c.decode([JSONValue].self) {
            self = .array(a)
        } else {
            self = .object(try c.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .string(let s): try c.encode(s)
        case .number(let n): try c.encode(n)
        case .bool(let b): try c.encode(b)
        case .null: try c.encodeNil()
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    /// The value as Foundation JSON, for a request body.
    public var any: Any {
        switch self {
        case .string(let s): return s
        case .number(let n): return n
        case .bool(let b): return b
        case .null: return NSNull()
        case .array(let a): return a.map(\.any)
        case .object(let o): return o.mapValues(\.any)
        }
    }

    public var string: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    public var object: [String: JSONValue]? {
        if case .object(let o) = self { return o }
        return nil
    }

    public var array: [JSONValue]? {
        if case .array(let a) = self { return a }
        return nil
    }

    public subscript(key: String) -> JSONValue? { object?[key] }
}

public enum Needs: String, Codable, Equatable, Sendable {
    case owner, agent, none
}

public enum ActionStyle: String, Codable, Equatable, Sendable {
    case primary, plain, destructive
}

public enum InputKind: String, Codable, Equatable, Sendable {
    case text, choice, toggle, number, secret
    case image
}

public enum ProgressMark: String, Codable, Equatable, Sendable {
    case ok, failed, stalled, done
}

public struct Action: Codable, Equatable, Sendable {
    public let label: String
    public let tool: String
    public let arguments: [String: JSONValue]
    public let style: ActionStyle
    public let proof: Bool

    public init(
        label: String, tool: String, arguments: [String: JSONValue] = [:],
        style: ActionStyle = .plain, proof: Bool = false
    ) {
        self.label = label
        self.tool = tool
        self.arguments = arguments
        self.style = style
        self.proof = proof
    }

    enum CodingKeys: String, CodingKey { case label, tool, arguments, style, proof }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        label = try c.decode(String.self, forKey: .label)
        tool = try c.decode(String.self, forKey: .tool)
        arguments = try c.decodeIfPresent([String: JSONValue].self, forKey: .arguments) ?? [:]
        style = (try? c.decode(ActionStyle.self, forKey: .style)) ?? .plain
        proof = try c.decodeIfPresent(Bool.self, forKey: .proof) ?? false
    }
}

public struct SeriesPoint: Codable, Equatable, Sendable {
    public let label: String
    public let n: Int
}

/// A field's value, decoded on `kind`; a kind this app doesn't know is `.unknown`.
public enum Value: Codable, Equatable, Sendable {
    case text(String)
    case code(String)
    case sha(String)
    case time(Int)
    case duration(Int)
    case count(Int)
    case link(text: String, href: String)
    case progress(stop: Int, of: Int, mark: ProgressMark)
    case series([SeriesPoint])
    case unknown

    enum CodingKeys: String, CodingKey {
        case kind, text, at, seconds, n, href, stop, of, mark, points
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try c.decode(String.self, forKey: .kind)
        do {
            switch kind {
            case "text": self = .text(try c.decode(String.self, forKey: .text))
            case "code": self = .code(try c.decode(String.self, forKey: .text))
            case "sha": self = .sha(try c.decode(String.self, forKey: .text))
            case "time": self = .time(try c.decode(Int.self, forKey: .at))
            case "duration": self = .duration(try c.decode(Int.self, forKey: .seconds))
            case "count": self = .count(try c.decode(Int.self, forKey: .n))
            case "link":
                self = .link(
                    text: try c.decode(String.self, forKey: .text),
                    href: try c.decode(String.self, forKey: .href))
            case "progress":
                self = .progress(
                    stop: try c.decode(Int.self, forKey: .stop),
                    of: try c.decode(Int.self, forKey: .of),
                    mark: (try? c.decode(ProgressMark.self, forKey: .mark)) ?? .ok)
            case "series": self = .series(try c.decode([SeriesPoint].self, forKey: .points))
            default: self = .unknown
            }
        } catch {
            self = .unknown
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .text(let t):
            try c.encode("text", forKey: .kind)
            try c.encode(t, forKey: .text)
        case .code(let t):
            try c.encode("code", forKey: .kind)
            try c.encode(t, forKey: .text)
        case .sha(let t):
            try c.encode("sha", forKey: .kind)
            try c.encode(t, forKey: .text)
        case .time(let at):
            try c.encode("time", forKey: .kind)
            try c.encode(at, forKey: .at)
        case .duration(let s):
            try c.encode("duration", forKey: .kind)
            try c.encode(s, forKey: .seconds)
        case .count(let n):
            try c.encode("count", forKey: .kind)
            try c.encode(n, forKey: .n)
        case .link(let text, let href):
            try c.encode("link", forKey: .kind)
            try c.encode(text, forKey: .text)
            try c.encode(href, forKey: .href)
        case .progress(let stop, let of, let mark):
            try c.encode("progress", forKey: .kind)
            try c.encode(stop, forKey: .stop)
            try c.encode(of, forKey: .of)
            try c.encode(mark, forKey: .mark)
        case .series(let points):
            try c.encode("series", forKey: .kind)
            try c.encode(points, forKey: .points)
        case .unknown:
            try c.encode("unknown", forKey: .kind)
        }
    }
}

public struct Field: Codable, Equatable, Sendable {
    public let label: String
    public let value: Value
}

public struct Section: Codable, Equatable, Sendable {
    public let heading: String?
    public let fields: [Field]
    public let text: String?

    enum CodingKeys: String, CodingKey { case heading, fields, text }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        heading = try c.decodeIfPresent(String.self, forKey: .heading)
        fields = try c.decodeIfPresent([Field].self, forKey: .fields) ?? []
        text = try c.decodeIfPresent(String.self, forKey: .text)
    }
}

public struct Row: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let title: String
    public let subtitle: String?
    public let badge: String?
    public let needs: Needs?
    public let open: Action?
}

public struct Input: Codable, Equatable, Sendable {
    public let key: String
    public let label: String
    public let input: InputKind
    public let required: Bool?
    public let help: String?
    public let options: [String]?
}

/// One of the four views a catalog tool answers with.
public enum ViewModel: Decodable, Equatable, Sendable {
    case list(title: String, items: [Row], next: String?)
    case card(title: String, body: String?, fields: [Field], actions: [Action])
    case detail(title: String, sections: [Section], actions: [Action])
    case form(title: String, fields: [Input], submit: Action)

    enum CodingKeys: String, CodingKey {
        case view, title, items, next, body, fields, actions, sections, submit
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let title = try c.decode(String.self, forKey: .title)
        switch try c.decode(String.self, forKey: .view) {
        case "list":
            self = .list(
                title: title,
                items: try c.decodeIfPresent([Row].self, forKey: .items) ?? [],
                next: try c.decodeIfPresent(String.self, forKey: .next))
        case "card":
            self = .card(
                title: title,
                body: try c.decodeIfPresent(String.self, forKey: .body),
                fields: try c.decodeIfPresent([Field].self, forKey: .fields) ?? [],
                actions: try c.decodeIfPresent([Action].self, forKey: .actions) ?? [])
        case "detail":
            self = .detail(
                title: title,
                sections: try c.decodeIfPresent([Section].self, forKey: .sections) ?? [],
                actions: try c.decodeIfPresent([Action].self, forKey: .actions) ?? [])
        case "form":
            self = .form(
                title: title,
                fields: try c.decodeIfPresent([Input].self, forKey: .fields) ?? [],
                submit: try c.decode(Action.self, forKey: .submit))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .view, in: c, debugDescription: "unknown view")
        }
    }

    public var title: String {
        switch self {
        case .list(let t, _, _), .card(let t, _, _, _), .detail(let t, _, _), .form(let t, _, _): return t
        }
    }
}
