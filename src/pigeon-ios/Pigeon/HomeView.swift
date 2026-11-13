// Home: the avatar for Profiles, Insights (Cost and Errors today), and Approvals with
// Accept edits / Auto; every card opens over Home. See the ADR's context.

import PigeonCore
import SwiftUI

enum Sheet: Identifiable {
    case profiles
    case chart(String)
    case approval(String)
    case auto

    var id: String {
        switch self {
        case .profiles: return "profiles"
        case .chart(let m): return "chart-\(m)"
        case .approval(let id): return "approval-\(id)"
        case .auto: return "auto"
        }
    }
}

struct HomeView: View {
    @EnvironmentObject private var store: Store
    @Binding var openApproval: String?
    @State private var sheet: Sheet?
    @State private var now = Int(Date().timeIntervalSince1970)

    private let tick = Timer.publish(every: 30, on: .main, in: .common).autoconnect()

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    header
                    if let error = store.error {
                        Text(error).font(.footnote).foregroundStyle(Theme.accent)
                    }
                    Text("Insights").font(Theme.display(22)).foregroundStyle(Theme.ink)
                    HStack(spacing: 10) {
                        tile("Cost", metric: "cost", stacks: store.home?.insights.cost)
                        tile("Errors", metric: "errors", stacks: store.home?.insights.errors)
                    }
                    approvals
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 32)
            }
            .refreshable { await store.refresh() }
        }
        .task { await store.refresh() }
        .onReceive(tick) { _ in now = Int(Date().timeIntervalSince1970) }
        .onChange(of: openApproval) { _, id in
            if let id { sheet = .approval(id) }
            openApproval = nil
        }
        .sheet(item: $sheet) { which in
            switch which {
            case .profiles: ProfilesSheet()
            case .chart(let metric): ChartSheet(metric: metric)
            case .approval(let id): ApprovalSheet(id: id)
            case .auto: AutoSheet()
            }
        }
    }

    private var header: some View {
        HStack {
            Button {
                sheet = .profiles
            } label: {
                Image("Avatar")
                    .resizable()
                    .frame(width: 40, height: 40)
                    .clipShape(Circle())
                    .overlay(Circle().stroke(Theme.card, lineWidth: 2))
                    .background(Circle().fill(Theme.shadow).offset(x: 2, y: 2))
            }
            .accessibilityLabel("Profile: Platform. Switch profile")
            Spacer()
        }
        .padding(.top, 8)
    }

    private func tile(_ title: String, metric: String, stacks: Stacks?) -> some View {
        Button {
            sheet = .chart(metric)
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(title).font(.subheadline.weight(.semibold))
                    Spacer()
                    Image(systemName: "arrow.up.left.and.arrow.down.right")
                        .font(.caption).foregroundStyle(Theme.muted)
                }
                Text(stacks.map { formatted($0.total, metric: metric) } ?? "—")
                    .font(Theme.display(22))
                MiniBars(stacks: stacks)
                    .frame(height: 48)
            }
            .foregroundStyle(Theme.ink)
            .padding(12)
            .block()
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Expand \(title)")
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

/// Auto's banner: when it ends, and that the owner tier still asks.
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
                        : "\(remaining(until: window.expires, now: now)) left · owner tier still asks"
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
