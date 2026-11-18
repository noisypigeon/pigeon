// The app's MCP session on /v0/app/mcp; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.

import Foundation
import PigeonCore

actor MCPClient {
    private let api: API
    private var session: String?
    private var nextID = 1

    init(api: API) {
        self.api = api
    }

    private func id() -> Int {
        defer { nextID += 1 }
        return nextID
    }

    private var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }

    private func start() async throws {
        let (_, header) = try await api.mcp(
            MCPRequest.initialize(id: id(), version: version), session: nil)
        session = header
    }

    /// Calls `tool`; a dropped session (404) initializes again and retries once.
    func call(_ tool: String, _ args: [String: JSONValue] = [:]) async throws -> MCPOutcome {
        if session == nil { try await start() }
        do {
            return try await send(tool, args)
        } catch let e as APIError where e.status == 404 {
            session = nil
            try await start()
            return try await send(tool, args)
        }
    }

    private func send(_ tool: String, _ args: [String: JSONValue]) async throws -> MCPOutcome {
        let (data, header) = try await api.mcp(
            MCPRequest.call(id: id(), tool: tool, arguments: args), session: session)
        if let header { session = header }
        return try MCPOutcome.decode(data)
    }
}
