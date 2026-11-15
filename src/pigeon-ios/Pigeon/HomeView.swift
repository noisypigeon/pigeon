// Approvals: the waiting cards with Accept edits / Auto, and the shared sheet, verb, bars and
// Auto banner. See the ADR's context.

import PigeonCore
import SwiftUI

enum Sheet: Identifiable {
    case instances
    case profile
    case chart(String)
    case approval(String)
    case auto

    var id: String {
        switch self {
        case .instances: return "instances"
        case .profile: return "profile"
        case .chart(let m): return "chart-\(m)"
        case .approval(let id): return "approval-\(id)"
        case .auto: return "auto"
        }
    }
}

struct ApprovalsView: View {
    @EnvironmentObject private var store: Store
    @Binding var sheet: Sheet?
    @State private var now = Int(Date().timeIntervalSince1970)

    private let tick = Timer.publish(every: 30, on: .main, in: .common).autoconnect()

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    TopBar(sheet: $sheet)
                    if let error = store.error {
                        Text(error).font(.footnote).foregroundStyle(Theme.accent)
                    }
                    approvals
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 32)
            }
            .refreshable { await store.refresh() }
        }
        .onReceive(tick) { _ in now = Int(Date().timeIntervalSince1970) }
    }

    private var approvals: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Approvals").font(Theme.display(22)).foregroundStyle(Theme.ink)
                Spacer()
                Text("\(store.home?.approvals.count ?? 0) waiting")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Theme.accent)
                    .padding(.horizontal, 10).padding(.vertical, 4)
                    .background(Capsule().fill(Theme.accentSoft))
            }
            .padding(.top, 8)
            modeSwitch
            if let auto = store.home?.auto {
                AutoBanner(window: auto, now: now)
            }
            ForEach(store.home?.approvals ?? []) { a in
                Button {
                    sheet = .approval(a.id)
                } label: {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(verb(a.operation) + " · !\(a.number)")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(Theme.accent)
                        Text(a.title)
                            .font(.callout.weight(.semibold))
                            .foregroundStyle(Theme.ink)
                            .multilineTextAlignment(.leading)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                    .block()
                }
                .buttonStyle(.plain)
            }
            if store.home?.approvals.isEmpty == true {
                Text("Nothing waiting.").font(.callout).foregroundStyle(Theme.muted)
            }
        }
    }

    private var modeSwitch: some View {
        let auto = store.home?.auto != nil
        return HStack(spacing: 4) {
            segment("Accept edits", on: !auto) {
                if auto { Task { _ = await store.endAuto() } }
            }
            segment("Auto", on: auto) {
                if !auto { sheet = .auto }
            }
        }
        .padding(3)
        .background(RoundedRectangle(cornerRadius: 12).fill(Theme.chip))
    }

    private func segment(_ title: String, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.subheadline.weight(.semibold))
                .frame(maxWidth: .infinity, minHeight: 36)
                .foregroundStyle(on ? Theme.ink : Theme.muted)
                .background(
                    RoundedRectangle(cornerRadius: 10).fill(on ? Theme.card : .clear)
                )
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
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

/// Auto's banner: when it ends, and that an administrator still approves the owner tier.
struct AutoBanner: View {
    @EnvironmentObject private var store: Store
    let window: AutoWindow
    let now: Int

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "timer").foregroundStyle(Theme.accent)
            VStack(alignment: .leading, spacing: 2) {
                Text(
                    window.state == "paused"
                        ? "Auto paused" : "Auto until \(clock(window.expires))"
                )
                .font(.subheadline.weight(.semibold))
                Text(
                    window.state == "paused"
                        ? (window.why ?? "")
                        : "\(remaining(until: window.expires, now: now)) left · an administrator approves the rest"
                )
                .font(Theme.mono(12))
                .foregroundStyle(Theme.muted)
            }
            Spacer()
            Button("End") { Task { _ = await store.endAuto() } }
                .font(.footnote.weight(.semibold))
                .padding(.horizontal, 12).frame(minHeight: 36)
                .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
        }
        .foregroundStyle(Theme.ink)
        .padding(14)
        .block(shadow: Theme.accent)
    }

    private func clock(_ t: Int) -> String {
        Date(timeIntervalSince1970: TimeInterval(t)).formatted(date: .omitted, time: .shortened)
    }
}
