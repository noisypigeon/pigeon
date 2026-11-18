// The owner's proof for an approval page: proof_challenge, the passkey, proof_submit; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §4.

import PigeonCore
import SwiftUI

struct ProofSheet: View {
    let client: MCPClient
    let page: String
    @Environment(\.dismiss) private var dismiss
    @State private var auth = Auth()
    @State private var challenge: MCPOutcome?
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
            let o = try await client.call("proof_challenge", ["page": .string(page)])
            if let e = o.error { error = e } else { challenge = o }
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
