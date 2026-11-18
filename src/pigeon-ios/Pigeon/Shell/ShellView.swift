// The Feed tab: four roots drawn from the view contract; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §1, §4.

import PigeonCore
import SwiftUI

enum ShellRoot: String, CaseIterable, Identifiable {
    case feed = "Feed"
    case home = "Home"
    case tracker = "Tracker"
    case changes = "Changes"

    var id: String { rawValue }

    var action: Action {
        switch self {
        case .feed:
            return Action(
                label: rawValue, tool: "feed.read",
                arguments: ["needs": .string("owner"), "open": .string("1")])
        case .home: return Action(label: rawValue, tool: "home.show")
        case .tracker: return Action(label: rawValue, tool: "tracker.show")
        case .changes: return Action(label: rawValue, tool: "change.list")
        }
    }
}

/// A tool's answer, loaded; shown by the shell and pushed on its stack.
struct Screen: Hashable {
    let action: Action
    var outcome: MCPOutcome?
    var error: String?

    func hash(into hasher: inout Hasher) { hasher.combine(action.tool) }
    static func == (a: Screen, b: Screen) -> Bool { a.action == b.action && a.outcome == b.outcome }
}

struct ShellView: View {
    @State private var root: ShellRoot = .feed
    @State private var client: MCPClient?
    @State private var current = Screen(action: ShellRoot.feed.action)
    @State private var loading = false
    @State private var path: [Screen] = []
    @State private var proofPage: ProofPage?

    struct ProofPage: Identifiable {
        let id = UUID()
        let path: String
    }

    var body: some View {
        NavigationStack(path: $path) {
            VStack(spacing: 0) {
                Picker("Root", selection: $root) {
                    ForEach(ShellRoot.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .padding()
                content(current)
            }
            .navigationDestination(for: Screen.self) { screen in content(screen) }
        }
        .sheet(item: $proofPage) { p in
            if let client { ProofSheet(client: client, page: p.path) }
        }
        .task(id: root) { await load(root.action) }
    }

    @ViewBuilder
    private func content(_ screen: Screen) -> some View {
        if let view = screen.outcome?.view {
            ViewModelView(model: view, act: { action in Task { await run(action) } })
                .refreshable { await load(screen.action) }
        } else if let error = screen.error ?? screen.outcome?.error {
            Text(error).foregroundStyle(.red).padding()
        } else if loading {
            ProgressView().frame(maxHeight: .infinity)
        } else {
            Text("Nothing to show.").foregroundStyle(Theme.muted).frame(maxHeight: .infinity)
        }
    }

    private func mcp() throws -> MCPClient {
        if let client { return client }
        let made = MCPClient(api: API(device: try DeviceKey.load()))
        client = made
        return made
    }

    /// Loads a root into the current screen.
    private func load(_ action: Action) async {
        loading = true
        defer { loading = false }
        var screen = Screen(action: action)
        do {
            let outcome = try await mcp().call(action.tool, action.arguments)
            screen.outcome = outcome
            if let page = outcome.pendingPath { proofPage = ProofPage(path: page) }
        } catch {
            screen.error = error.localizedDescription
        }
        current = screen
    }

    /// Runs an action and pushes its answer, or opens the proof sheet for a pending approval.
    private func run(_ action: Action) async {
        var screen = Screen(action: action)
        do {
            let outcome = try await mcp().call(action.tool, action.arguments)
            if let page = outcome.pendingPath {
                proofPage = ProofPage(path: page)
                return
            }
            screen.outcome = outcome
        } catch {
            screen.error = error.localizedDescription
        }
        path.append(screen)
    }
}
