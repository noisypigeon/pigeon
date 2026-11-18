// The owner's proof for an approval page: proof_challenge, the passkey, proof_submit; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §4.

import PigeonCore
import SwiftUI

struct ProofSheet: View {
    let client: MCPClient
    let page: String
    var purpose: String? = nil
    var target: String? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var auth = Auth()
    @State private var challenge: MCPOutcome?
    @State private var held: JSONValue?
    @State private var stage: MCPOutcome?
    @State private var result: MCPOutcome?
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Group {
                if let result {
                    if let view = result.view {
                        ViewModelView(model: view, act: { _ in })
                    } else {
                        Text(result.error ?? "Done.").padding()
                    }
                } else if let view = stage?.view, shows(view) {
                    ViewModelView(model: view, act: { a in Task { await act(a) } })
                } else if let data = challenge?.data {
                    List {
                        SwiftUI.Section("Approves") {
                            row("Operation", data["operation"]?.string)
                            row("Target", data["target"]?.string)
                            row("Head", data["head"]?.string.map { String($0.prefix(12)) })
                        }
                        SwiftUI.Section {
                            Button("Approve with passkey") { Task { await approve(data) } }
                                .buttonStyle(.borderedProminent)
                                .disabled(busy)
                        }
                        if let error { SwiftUI.Section { Text(error).foregroundStyle(.red) } }
                    }
                } else if let error {
                    Text(error).foregroundStyle(.red).padding()
                } else {
                    ProgressView()
                }
            }
            .navigationTitle("Approval")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(result == nil ? "Cancel" : "Done") { dismiss() }
                }
            }
        }
        .task { await load() }
    }

    private func row(_ label: String, _ value: String?) -> some View {
        LabeledContent(label) { Text(value ?? "–").font(Theme.mono(13)) }
    }

    private func load() async {
        do {
            let o = try await client.call(
                "proof_challenge",
                purpose.map { p in
                    var a: [String: JSONValue] = ["purpose": .string(p)]
                    if let target { a["target"] = .string(target) }
                    return a
                } ?? ["page": .string(page)])
            if let e = o.error { error = e } else { challenge = o; held = o.data; stage = o }
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Whether the view on show carries a proof action or a tool beyond approval.list.
    private func shows(_ view: ViewModel) -> Bool {
        actions(of: view).contains { $0.proof || $0.tool != "approval.list" }
    }

    private func actions(of view: ViewModel) -> [Action] {
        switch view {
        case .card(_, _, _, let actions): return actions
        case .form(_, _, let submit): return [submit]
        case .detail(_, _, let actions): return actions
        case .list(_, let items, _): return items.compactMap(\.open)
        }
    }

    /// Runs an action; a proof action is signed with the latest challenge.
    private func act(_ a: Action) async {
        if a.tool == "approval.list" { dismiss(); return }
        busy = true
        defer { busy = false }
        do {
            var args = a.arguments
            if a.proof {
                guard let data = held, let text = data["challenge"]?.string,
                    let rp = data["rp"]?.string
                else {
                    error = "The challenge couldn't be read."
                    return
                }
                let ids = data["ids"]?.array?.compactMap(\.string) ?? []
                let options: [String: JSONValue] = [
                    "challenge": .string(text), "ids": .array(ids.map(JSONValue.string)),
                    "rp": .string(rp),
                ]
                let bytes = try JSONEncoder().encode(JSONValue.object(options))
                let answer = try await auth.prove(JSONDecoder().decode(PasskeyChallenge.self, from: bytes))
                args["credential"] = .string(answer.id)
                args["client_data"] = .string(answer.client)
                args["authenticator_data"] = .string(answer.authenticator)
                args["signature"] = .string(answer.signature)
            }
            let o = try await client.call(a.tool, args)
            if let data = o.data, data["challenge"]?.string != nil, data["rp"]?.string != nil {
                held = data
            }
            stage = o
            if a.proof { result = o }
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func approve(_ data: JSONValue) async {
        guard let text = data["challenge"]?.string, let rp = data["rp"]?.string else {
            error = "The challenge couldn't be read."
            return
        }
        busy = true
        defer { busy = false }
        let ids = data["ids"]?.array?.compactMap(\.string) ?? []
        do {
            let options: [String: JSONValue] = ["challenge": .string(text), "ids": .array(ids.map(JSONValue.string)), "rp": .string(rp)]
            let bytes = try JSONEncoder().encode(JSONValue.object(options))
            let answer = try await auth.prove(JSONDecoder().decode(PasskeyChallenge.self, from: bytes))
            let operation = data["operation"]?.string ?? ""
            result = try await client.call(
                "proof_submit",
                [
                    "page": .string(page),
                    "fields": .object(["operation": .string(operation)]),
                    "credential": .string(answer.id),
                    "client_data": .string(answer.client),
                    "authenticator_data": .string(answer.authenticator),
                    "signature": .string(answer.signature),
                ])
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// An approval opened by its id: its page from approval.list, in the proof sheet; see the feed-view ADR's R10b amendment.
struct ApprovalProof: View {
    @EnvironmentObject private var store: Store
    let id: String
    @State private var page: String?
    @State private var missing = false

    var body: some View {
        Group {
            if let page, let client = try? store.mcp() {
                ProofSheet(client: client, page: page)
            } else if missing {
                Text("This approval is no longer waiting.").padding()
            } else {
                ProgressView()
            }
        }
        .task {
            page = await store.approvalPage(id)
            missing = page == nil
        }
    }
}
