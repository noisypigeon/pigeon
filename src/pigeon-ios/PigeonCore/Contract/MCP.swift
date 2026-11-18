// The MCP messages the app sends to /v0/app/mcp and what it reads back; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.

import Foundation

/// A JSON-RPC `error` member.
public struct MCPError: Error, LocalizedError, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var errorDescription: String? { message }
}

public enum MCPRequest {
    public static let protocolVersion = "2025-06-18"

    public static func initialize(id: Int, version: String = "0") -> [String: Any] {
        [
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": [
                "protocolVersion": protocolVersion,
                "capabilities": [String: Any](),
                "clientInfo": ["name": "pigeon-ios", "version": version],
            ] as [String: Any],
        ]
    }

    /// `tools/call`; a dotted operation becomes the tool's name with underscores.
    public static func call(id: Int, tool: String, arguments: [String: JSONValue]) -> [String: Any] {
        [
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": [
                "name": tool.replacingOccurrences(of: ".", with: "_"),
                "arguments": arguments.mapValues(\.any),
            ] as [String: Any],
        ]
    }
}

/// A tools/call answer: its view, its data, its error, and any approval page it points at.
public struct MCPOutcome: Equatable, Sendable {
    public let view: ViewModel?
    public let data: JSONValue?
    public let error: String?
    public let pendingPath: String?

    public init(view: ViewModel?, data: JSONValue?, error: String?, pendingPath: String?) {
        self.view = view
        self.data = data
        self.error = error
        self.pendingPath = pendingPath
    }

    static let pages = ["/approve/", "/mandate/", "/lane/", "/grants/", "/ci-window/", "/platform/"]

    /// Reads a JSON-RPC response body.
    public static func decode(_ data: Data) throws -> MCPOutcome {
        guard let root = try? JSONDecoder().decode(JSONValue.self, from: data) else {
            throw MCPError("The answer wasn't JSON.")
        }
        if let e = root["error"] {
            throw MCPError(e["message"]?.string ?? "The control plane refused the call.")
        }
        guard let result = root["result"] else { throw MCPError("The answer had no result.") }
        return outcome(result)
    }

    /// Reads a tools/call `result`.
    public static func outcome(_ result: JSONValue) -> MCPOutcome {
        var view: ViewModel?
        if let meta = result["_meta"]?["dev.pigeon/view"],
            let bytes = try? JSONEncoder().encode(meta)
        {
            view = try? JSONDecoder().decode(ViewModel.self, from: bytes)
        }
        let structured = result["structuredContent"]
        let texts = (result["content"]?.array ?? []).compactMap { $0["text"]?.string }
        var error: String?
        if case .bool(true)? = result["isError"] { error = texts.joined(separator: "\n") }
        var path: String?
        if let link = structured?["result"]?["pending"]?["link"]?.string {
            path = pathAndQuery(link)
        } else {
            for text in texts {
                for word in text.split(whereSeparator: { $0.isWhitespace || "\"<>()".contains($0) }) {
                    guard word.hasPrefix("https://"), let p = pathAndQuery(String(word)),
                        pages.contains(where: { p.hasPrefix($0) })
                    else { continue }
                    path = p
                    break
                }
                if path != nil { break }
            }
        }
        return MCPOutcome(view: view, data: structured, error: error, pendingPath: path)
    }

    static func pathAndQuery(_ link: String) -> String? {
        guard let u = URLComponents(string: link) else { return nil }
        return u.path + (u.query.map { "?\($0)" } ?? "")
    }
}
