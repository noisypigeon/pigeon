// The shared sheet route, verb, bars, Auto's banner and the owner's item card, which the Now
// tab shows; see the standing Auto ADR §5 and .claude/decisions/2026-11-15-runtime-ios-app-now-tab.md §5.

import PigeonCore
import SwiftUI

enum Sheet: Identifiable {
    case instances
    case profile
    case chart(String)
    case approval(String)

    var id: String {
        switch self {
        case .instances: return "instances"
        case .profile: return "profile"
        case .chart(let m): return "chart-\(m)"
        case .approval(let id): return "approval-\(id)"
        }
    }
}

func verb(_ operation: String) -> String {
    switch operation {
    case "change.authorize": return "Authorize plan"
    case "change.approve": return "Approve head"
    case "change.merge": return "Merge"
    case "change.close": return "Close"
    default: return operation
    }
}

/// The two tiles' small stacked bars, by source.
struct MiniBars: View {
    let stacks: Stacks?

    var body: some View {
        GeometryReader { geo in
            let buckets = stacks?.buckets ?? []
            HStack(alignment: .bottom, spacing: 2) {
                ForEach(buckets, id: \.at) { b in
                    VStack(spacing: 1) {
                        Spacer(minLength: 0)
                        ForEach(
                            segments(of: b, among: buckets, height: geo.size.height - 2).reversed(),
                            id: \.name
                        ) { s in
                            Rectangle().fill(Theme.source(s.name)).frame(height: s.height)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.ink).frame(height: 2) }
        }
        .accessibilityHidden(true)
    }
}

/// Auto's banner: on, held or paused, with Pause or Resume; it never expires, and an
/// administrator still approves the owner tier and guarded paths.
struct AutoBanner: View {
    @EnvironmentObject private var store: Store
    @State private var proving: Proving?
    let window: AutoWindow
    let now: Int

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: window.state == "active" ? "bolt.fill" : "pause.circle")
                .foregroundStyle(Theme.accent)
            VStack(alignment: .leading, spacing: 2) {
                Text(heading).font(.subheadline.weight(.semibold))
                Text(detail)
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.muted)
            }
            Spacer()
            if window.state == "paused" {
                Button("Resume") { proving = Proving(purpose: "auto") }
                    .font(.footnote.weight(.semibold))
                    .padding(.horizontal, 12).frame(minHeight: 36)
                    .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
                    .disabled(store.busy)
            } else {
                Button("Pause") { proving = Proving(purpose: "auto-end") }
                    .font(.footnote.weight(.semibold))
                    .padding(.horizontal, 12).frame(minHeight: 36)
                    .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
                    .disabled(store.busy)
            }
        }
        .foregroundStyle(Theme.ink)
        .padding(14)
        .block(shadow: Theme.accent)
        .sheet(item: $proving, onDismiss: { Task { await store.refresh() } }) { p in
            if let client = try? store.mcp() { ProofSheet(client: client, page: "", purpose: p.purpose) }
        }
    }

    private var heading: String {
        switch window.state {
        case "held": return "Auto held"
        case "paused": return "Auto paused"
        default: return "Auto"
        }
    }

    private var detail: String {
        switch window.state {
        case "held", "paused": return window.why ?? ""
        default:
            return "\(window.rate(now: now)) merges in 6h · an administrator approves the owner tier"
        }
    }
}

/// Auto is off: the owner resumes it with a passkey.
struct AutoOff: View {
    @EnvironmentObject private var store: Store
    @State private var proving: Proving?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "pause.circle").foregroundStyle(Theme.muted)
            VStack(alignment: .leading, spacing: 2) {
                Text("Auto paused").font(.subheadline.weight(.semibold))
                Text("Each approval asks you until you resume it")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.muted)
            }
            Spacer()
            Button("Resume") { proving = Proving(purpose: "auto") }
                .font(.footnote.weight(.semibold))
                .padding(.horizontal, 12).frame(minHeight: 36)
                .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
                .disabled(store.busy)
        }
        .foregroundStyle(Theme.ink)
        .padding(14)
        .block()
        .sheet(item: $proving, onDismiss: { Task { await store.refresh() } }) { p in
            if let client = try? store.mcp() { ProofSheet(client: client, page: "", purpose: p.purpose) }
        }
    }
}

/// A proof the Auto banner asks for: `proof.challenge`'s purpose.
struct Proving: Identifiable {
    let purpose: String
    var id: String { purpose }
}

/// An owner item: a decision with its options and free text, or an action to mark done.
struct ItemCard: View {
    @EnvironmentObject private var store: Store
    let item: OwnerItem
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(
                (item.kind == "decision" ? "Decision" : "Action")
                    + (item.change.map { " · !\($0)" } ?? "")
            )
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Theme.accent)
            Text(item.title).font(.callout.weight(.semibold)).foregroundStyle(Theme.ink)
            Text(item.detail).font(.footnote).foregroundStyle(Theme.ink2)
            ForEach(item.options ?? [], id: \.self) { option in
                Button(option) { send(option) }
                    .font(.footnote.weight(.semibold))
                    .frame(maxWidth: .infinity, minHeight: 36)
                    .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
                    .buttonStyle(.plain)
            }
            if item.kind == "action" {
                Button("Done") { send("done") }
                    .font(.footnote.weight(.semibold))
                    .frame(maxWidth: .infinity, minHeight: 36)
                    .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
                    .buttonStyle(.plain)
            }
            HStack {
                TextField(item.kind == "action" ? "Note" : "Answer", text: $text)
                    .font(.footnote)
                Button("Send") { send(text) }
                    .font(.footnote.weight(.semibold))
                    .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .disabled(store.busy)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .block(shadow: Theme.accent)
    }

    private func send(_ answer: String) {
        Task { _ = await store.answer(item: item.id, with: answer) }
    }
}
