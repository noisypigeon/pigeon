// The tracker: each session's changes on a ten-stop rail, live through /v0/app/events; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md's 2026-11-15 amendment.

import PigeonCore
import SwiftUI

struct TrackerView: View {
    @EnvironmentObject private var store: Store
    @Binding var sheet: Sheet?
    @Binding var tab: AppTab
    @State private var filter: TrackerFilter = .all
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
                    title
                    chips
                    if let auto = store.home?.auto {
                        AutoBanner(window: auto, now: now)
                    }
                    ForEach(shown) { session in
                        SessionCard(session: session, now: now, sheet: $sheet, tab: $tab)
                    }
                    if shown.isEmpty {
                        Text("Nothing running.").font(.callout).foregroundStyle(Theme.muted)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 32)
            }
            .refreshable { await store.loadTracker() }
        }
        .task { await store.follow() }
        .onReceive(tick) { _ in now = Int(Date().timeIntervalSince1970) }
    }

    private var shown: [TrackedSession] {
        guard let tracker = store.tracker else { return [] }
        return filtered(tracker, by: filter)
    }

    private var title: some View {
        let sessions = store.tracker?.sessions ?? []
        let changes = sessions.reduce(0) { $0 + $1.changes.count }
        return HStack(alignment: .firstTextBaseline) {
            Text("Tracker").font(Theme.display(22)).foregroundStyle(Theme.ink)
            Spacer()
            Text("\(sessions.count) sessions · \(changes) changes")
                .font(Theme.mono(12)).foregroundStyle(Theme.muted)
        }
        .padding(.top, 8)
    }

    private var chips: some View {
        let counts = store.tracker?.counts
        return HStack(spacing: 8) {
            if let n = counts?.owner, n > 0 { chip("\(n) need you", .owner) }
            if let n = counts?.stalled, n > 0 { chip("\(n) stalled", .stalled) }
            if let n = counts?.provider, n > 0 { chip("\(n) on a provider", .provider) }
        }
    }

    private func chip(_ text: String, _ f: TrackerFilter) -> some View {
        let on = filter == f
        return Button {
            filter = on ? .all : f
        } label: {
            Text(text)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(on ? Theme.onAccent : Theme.ink2)
                .padding(.horizontal, 10).frame(minHeight: 32)
                .background(Capsule().fill(on ? Theme.accent : Theme.chip))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// One session and its changes.
struct SessionCard: View {
    @EnvironmentObject private var store: Store
    let session: TrackedSession
    let now: Int
    @Binding var sheet: Sheet?
    @Binding var tab: AppTab

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            ForEach(session.changes) { change in
                changeRow(change)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .block(shadow: shadow)
    }

    private var shadow: Color {
        if session.changes.contains(where: { $0.needs == "owner" }) { return Theme.accent }
        if session.changes.contains(where: { $0.mark == .stalled }) { return Theme.warn }
        return Theme.shadow
    }

    private var header: some View {
        let opus = session.model?.lowercased() == "opus"
        return HStack(spacing: 8) {
            Text(modelBadge(session.model))
                .font(Theme.mono(11, .semibold))
                .foregroundStyle(opus ? Theme.background : Theme.ink)
                .padding(.horizontal, 8).padding(.vertical, 3)
                .background(RoundedRectangle(cornerRadius: 6).fill(opus ? Theme.ink : Theme.chip))
            Text(title)
            .font(Theme.mono(12)).foregroundStyle(Theme.ink2)
            Spacer()
            Text(session.listening ? "listening" : "silent")
                .font(Theme.mono(12, .semibold))
                .foregroundStyle(session.listening ? Theme.good : Theme.warn)
        }
    }

    private var title: String {
        var text: String = session.role?.capitalized ?? "Session"
        text += " " + shortLabel(session.label)
        if session.role == "executor", let planner = session.planner {
            text += " ← " + shortLabel(planner)
        }
        return text
    }

    private func changeRow(_ c: TrackedChange) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("!\(c.number) \(c.title)")
                .font(.callout.weight(.semibold))
                .foregroundStyle(Theme.ink)
                .multilineTextAlignment(.leading)
            if let b = c.brief {
                Text(briefLine(b)).font(Theme.mono(11)).foregroundStyle(Theme.muted)
            }
            Rail(change: c)
            HStack(spacing: 8) {
                Text(c.stage).font(Theme.mono(12, .semibold)).foregroundStyle(Theme.ink)
                if let since = c.since {
                    Text(elapsed(since: since, now: now))
                        .font(Theme.mono(12)).foregroundStyle(Theme.muted)
                }
                Spacer()
                needs(c)
            }
        }
    }

    private func briefLine(_ b: BriefRef) -> String {
        let name = b.adr.split(separator: "/").last.map(String.init) ?? b.adr
        let adr = name.hasSuffix(".md") ? String(name.dropLast(3)) : name
        return [b.id, adr, b.step].joined(separator: " · ")
    }

    @ViewBuilder
    private func needs(_ c: TrackedChange) -> some View {
        let text = needsChip(c)
        if c.needs == "owner" {
            Button {
                if let a = store.home?.approvals.first(where: { $0.number == c.number }) {
                    sheet = .approval(a.id)
                } else {
                    tab = .approvals
                }
            } label: {
                capsule(text, fill: Theme.accent, ink: Theme.onAccent)
            }
            .buttonStyle(.plain)
        } else if !text.isEmpty {
            capsule(text, fill: Theme.chip, ink: Theme.ink2)
        }
    }

    private func capsule(_ text: String, fill: Color, ink: Color) -> some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .foregroundStyle(ink)
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(Capsule().fill(fill))
    }
}

/// A change's ten-stop rail.
struct Rail: View {
    let change: TrackedChange

    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<10, id: \.self) { i in
                segment(i)
            }
        }
        .frame(height: 8)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(railLabel(change))
    }

    @ViewBuilder
    private func segment(_ i: Int) -> some View {
        if i < change.stop {
            RoundedRectangle(cornerRadius: 2).fill(Theme.ink)
        } else if i == change.stop {
            atStop
        } else {
            RoundedRectangle(cornerRadius: 2).fill(Theme.chip)
        }
    }

    @ViewBuilder
    private var atStop: some View {
        switch change.mark {
        case .ok:
            RoundedRectangle(cornerRadius: 2).fill(Theme.accent)
                .overlay(RoundedRectangle(cornerRadius: 2).stroke(Theme.ink, lineWidth: 1.5))
        case .stalled:
            RoundedRectangle(cornerRadius: 2).fill(Theme.chip)
                .overlay(Stripes().fill(Theme.warn).clipShape(RoundedRectangle(cornerRadius: 2)))
        case .failed:
            RoundedRectangle(cornerRadius: 2).fill(Theme.warn)
                .overlay(Image(systemName: "xmark").font(.system(size: 6, weight: .bold))
                    .foregroundStyle(Theme.onAccent))
        case .done:
            RoundedRectangle(cornerRadius: 2).fill(Theme.ink)
        case .held:
            RoundedRectangle(cornerRadius: 2).fill(Theme.accent)
                .overlay(Image(systemName: "pause.fill").font(.system(size: 6))
                    .foregroundStyle(Theme.onAccent))
        }
    }
}

/// Diagonal stripes for a stalled stop.
struct Stripes: Shape {
    func path(in rect: CGRect) -> Path {
        var p = Path()
        var x = -rect.height
        while x < rect.width {
            p.move(to: CGPoint(x: x, y: rect.height))
            p.addLine(to: CGPoint(x: x + 2, y: rect.height))
            p.addLine(to: CGPoint(x: x + rect.height + 2, y: 0))
            p.addLine(to: CGPoint(x: x + rect.height, y: 0))
            p.closeSubpath()
            x += 4
        }
        return p
    }
}
