// The tracker: each session's changes on a ten-stop rail, live through /v0/app/events; silent
// sessions hidden by default, and a change's sheet a tap away. See
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md's 2026-11-15 amendments.

import PigeonCore
import SwiftUI

/// The change whose sheet is open.
struct OpenedChange: Identifiable {
    let id: Int
}

struct TrackerView: View {
    @EnvironmentObject private var store: Store
    @Binding var sheet: Sheet?
    @Binding var tab: AppTab
    @State private var filter: TrackerFilter = .all
    @State private var showSilent = false
    @State private var opened: OpenedChange?
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
                        SessionCard(
                            session: session, now: now, sheet: $sheet, tab: $tab, opened: $opened)
                    }
                    if shown.isEmpty {
                        Text("Nothing running.").font(.callout).foregroundStyle(Theme.muted)
                    }
                    hiddenLine
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 32)
            }
            .refreshable { await store.loadTracker() }
        }
        .task { await store.follow() }
        .onReceive(tick) { _ in now = Int(Date().timeIntervalSince1970) }
        .sheet(item: $opened) { o in
            ChangeSheet(number: o.id, now: now, sheet: $sheet, tab: $tab)
                .environmentObject(store)
        }
    }

    private var shown: [TrackedSession] {
        guard let tracker = store.tracker else { return [] }
        return visible(tracker, by: filter, showSilent: showSilent)
    }

    private var silent: Int {
        store.tracker.map(silentCount) ?? 0
    }

    private var title: some View {
        let changes = shown.reduce(0) { $0 + $1.changes.count }
        return HStack(alignment: .firstTextBaseline) {
            Text("Tracker").font(Theme.display(22)).foregroundStyle(Theme.ink)
            Spacer()
            Text("\(shown.count) sessions · \(changes) changes")
                .font(Theme.mono(12)).foregroundStyle(Theme.muted)
        }
        .padding(.top, 8)
    }

    private var chips: some View {
        let counts = store.tracker?.counts
        return FlowRow {
            if let n = counts?.owner, n > 0 { chip("\(n) need you", .owner) }
            if let n = counts?.stalled, n > 0 { chip("\(n) stalled", .stalled) }
            if let n = counts?.provider, n > 0 { chip("\(n) on a provider", .provider) }
            if silent > 0 { silentChip }
        }
    }

    private func chip(_ text: String, _ f: TrackerFilter) -> some View {
        let on = filter == f
        return Button {
            filter = on ? .all : f
        } label: {
            chipLabel(Text(text), on: on)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    private var silentChip: some View {
        Button {
            showSilent.toggle()
        } label: {
            let glyph = Image(systemName: showSilent ? "circle.inset.filled" : "circle.dashed")
            chipLabel(Text("\(glyph) \(silent) silent"), on: showSilent)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(silent) silent sessions")
        .accessibilityValue(showSilent ? "shown" : "hidden")
        .accessibilityAddTraits(showSilent ? .isSelected : [])
    }

    private func chipLabel(_ text: Text, on: Bool) -> some View {
        text
            .font(.footnote.weight(.semibold))
            .foregroundStyle(on ? Theme.onAccent : Theme.ink2)
            .padding(.horizontal, 10).frame(minHeight: 32)
            .background(Capsule().fill(on ? Theme.accent : Theme.chip))
    }

    @ViewBuilder
    private var hiddenLine: some View {
        if !showSilent, silent > 0 {
            Button {
                showSilent = true
            } label: {
                (Text("\(silent) silent \(silent == 1 ? "session" : "sessions") hidden · ")
                    .foregroundStyle(Theme.muted)
                    + Text("Show").foregroundStyle(Theme.accent).fontWeight(.semibold))
                    .font(.subheadline)
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(.plain)
        }
    }
}

/// Chips that wrap onto a new line when the row is full.
struct FlowRow: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(subviews, width: proposal.width ?? .infinity)
        let height = rows.last.map { $0.y + $0.height } ?? 0
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0, height: height)
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        for row in arrange(subviews, width: bounds.width) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: bounds.minY + row.y), proposal: .unspecified)
                x += size.width + spacing
            }
        }
    }

    private struct Row {
        var items: [Int] = []
        var y: CGFloat = 0
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(.unspecified)
            if !row.items.isEmpty, row.width + spacing + size.width > width {
                rows.append(row)
                row = Row(y: row.y + row.height + spacing)
            }
            row.width += (row.items.isEmpty ? 0 : spacing) + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
        }
        if !row.items.isEmpty { rows.append(row) }
        return rows
    }
}

/// One session and its changes, glanceable: a pill, the title in full and the rail per change.
struct SessionCard: View {
    @EnvironmentObject private var store: Store
    let session: TrackedSession
    let now: Int
    @Binding var sheet: Sheet?
    @Binding var tab: AppTab
    @Binding var opened: OpenedChange?

    var body: some View {
        let quiet = isSilent(session)
        VStack(alignment: .leading, spacing: 10) {
            header(quiet)
            ForEach(session.changes) { change in
                if change.mark == .done {
                    doneRow(change)
                } else {
                    row(change)
                }
            }
            if session.changes.isEmpty {
                Text("No open change").font(.subheadline).foregroundStyle(Theme.muted)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 14).padding(.vertical, 12)
        .modifier(CardStyle(quiet: quiet, shadow: shadow))
    }

    private var shadow: Color {
        if session.changes.contains(where: { $0.needs == "owner" }) { return Theme.accent }
        if session.changes.contains(where: { $0.mark == .stalled }) { return Theme.warn }
        return Theme.shadow
    }

    private func header(_ quiet: Bool) -> some View {
        HStack(spacing: 8) {
            ModelBadge(model: session.model, small: true)
            Spacer()
            if quiet {
                Text("SILENT").font(Theme.mono(11, .semibold)).foregroundStyle(Theme.muted)
            }
            ListeningDot(listening: session.listening)
        }
    }

    private func row(_ c: TrackedChange) -> some View {
        let status = rowStatus(c)
        let split = scopeAndSubject(c.title)
        return VStack(alignment: .leading, spacing: 8) {
            Button {
                opened = OpenedChange(id: c.number)
            } label: {
                VStack(alignment: .leading, spacing: 7) {
                    HStack(spacing: 8) {
                        StatusPill(status: status)
                        if !status.note.isEmpty {
                            Text(status.note).font(Theme.mono(12)).foregroundStyle(Theme.muted)
                        }
                        Spacer(minLength: 4)
                        if let since = c.since {
                            Text(elapsed(since: since, now: now))
                                .font(Theme.mono(12)).foregroundStyle(Theme.muted)
                        }
                        Image(systemName: "chevron.right")
                            .font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted)
                    }
                    TitleLine(tag: split.tag, subject: split.subject)
                    Rail(change: c, height: 6)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)
            .accessibilityHint("Opens the change")
            if c.needs == "owner" {
                ReviewButton(number: c.number, sheet: $sheet, tab: $tab)
            }
        }
    }

    private func doneRow(_ c: TrackedChange) -> some View {
        Button {
            opened = OpenedChange(id: c.number)
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                StatusPill(status: rowStatus(c))
                Text("!\(c.number) \(scopeAndSubject(c.title).subject)")
                    .font(.subheadline).foregroundStyle(Theme.muted)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 4)
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
    }
}

/// A title's scope tag and its subject, wrapped in full.
struct TitleLine: View {
    let tag: String?
    let subject: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            if let tag {
                ScopeTag(text: tag)
            }
            Text(subject)
                .font(.callout.weight(.semibold))
                .foregroundStyle(Theme.ink)
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A change's scope, as a small tag.
struct ScopeTag: View {
    let text: String

    var body: some View {
        Text(text)
            .font(Theme.mono(11, .semibold)).foregroundStyle(Theme.ink2)
            .padding(.horizontal, 6).padding(.vertical, 1)
            .background(RoundedRectangle(cornerRadius: 5).fill(Theme.chip))
    }
}

/// A card's surface: the shadowed block, or an outline for a silent session.
struct CardStyle: ViewModifier {
    let quiet: Bool
    let shadow: Color

    @ViewBuilder
    func body(content: Content) -> some View {
        if quiet {
            content.overlay(RoundedRectangle(cornerRadius: 14).stroke(Theme.line, lineWidth: 1.5))
        } else {
            content.block(shadow: shadow)
        }
    }
}

/// The session's class as a badge.
struct ModelBadge: View {
    let model: String?
    var small = false

    var body: some View {
        let opus = model?.lowercased() == "opus"
        Text(modelBadge(model))
            .font(Theme.mono(small ? 10 : 11, .semibold))
            .foregroundStyle(opus ? Theme.background : Theme.ink)
            .padding(.horizontal, small ? 6 : 8).padding(.vertical, small ? 2 : 3)
            .background(RoundedRectangle(cornerRadius: 5).fill(opus ? Theme.ink : Theme.chip))
    }
}

/// Filled when the session is listening, hollow when not.
struct ListeningDot: View {
    let listening: Bool

    var body: some View {
        Group {
            if listening {
                Circle().fill(Theme.good)
            } else {
                Circle().stroke(Theme.muted, lineWidth: 1.5)
            }
        }
        .frame(width: 8, height: 8)
        .accessibilityLabel(listening ? "listening" : "not listening")
    }
}

/// A row's status: a glyph and a word; only `Needs you` is filled in the accent.
struct StatusPill: View {
    let status: RowStatus

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: glyph).font(.system(size: 9, weight: .bold))
            Text(status.word).font(.footnote.weight(.bold))
        }
        .lineLimit(1)
        .fixedSize()
        .foregroundStyle(ink)
        .padding(.horizontal, 10).padding(.vertical, 3)
        .background(Capsule().fill(fill))
        .overlay(Capsule().stroke(stroke, lineWidth: 1.5))
    }

    private var glyph: String {
        switch status.kind {
        case .owner: return "exclamationmark"
        case .failed: return "xmark"
        case .stalled: return "circle.dashed"
        case .held: return "pause.fill"
        case .done: return "checkmark"
        case .moving: return "circle.fill"
        }
    }

    private var fill: Color {
        switch status.kind {
        case .owner: return Theme.accent
        case .failed: return Theme.warn
        case .held: return Theme.accentSoft
        case .moving: return Theme.chip
        case .stalled, .done: return .clear
        }
    }

    private var ink: Color {
        switch status.kind {
        case .owner, .failed: return Theme.onAccent
        case .held: return Theme.accent
        case .stalled: return Theme.warn
        case .done: return Theme.good
        case .moving: return Theme.ink
        }
    }

    private var stroke: Color {
        switch status.kind {
        case .stalled: return Theme.warn
        case .done: return Theme.good
        default: return .clear
        }
    }
}

/// Opens the change's approval, or the Approvals tab when it isn't listed yet.
struct ReviewButton: View {
    @EnvironmentObject private var store: Store
    let number: Int
    @Binding var sheet: Sheet?
    @Binding var tab: AppTab

    var body: some View {
        Button {
            if let a = store.home?.approvals.first(where: { $0.number == number }) {
                sheet = .approval(a.id)
            } else {
                tab = .approvals
            }
        } label: {
            Text("Review and approve")
                .font(.callout.weight(.semibold))
                .foregroundStyle(Theme.onAccent)
                .padding(.horizontal, 18).frame(minHeight: 44)
                .background(Capsule().fill(Theme.accent))
        }
        .buttonStyle(.plain)
    }
}

/// A change's sheet: its title, progress, what's next, its session, and a link into Claude.
struct ChangeSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    let number: Int
    let now: Int
    @Binding var sheet: Sheet?
    @Binding var tab: AppTab

    private var found: (session: TrackedSession, change: TrackedChange)? {
        for s in store.tracker?.sessions ?? [] {
            if let c = s.changes.first(where: { $0.number == number }) { return (s, c) }
        }
        return nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Text("!\(number)").font(Theme.mono(13, .semibold)).foregroundStyle(Theme.muted)
                if let tag = found.flatMap({ scopeAndSubject($0.change.title).tag }) {
                    ScopeTag(text: tag)
                }
                Spacer()
                Button("Done") { dismiss() }
                    .font(.body.weight(.semibold)).foregroundStyle(Theme.accent)
                    .frame(minHeight: 44)
            }
            if let f = found {
                content(f.session, f.change)
            } else {
                Text("This change has left the tracker.")
                    .font(.callout).foregroundStyle(Theme.muted)
                Spacer()
            }
        }
        .padding(.horizontal, 20).padding(.top, 16).padding(.bottom, 20)
        .background(Theme.card)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
    }

    @ViewBuilder
    private func content(_ s: TrackedSession, _ c: TrackedChange) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(scopeAndSubject(c.title).subject)
                    .font(.title3.weight(.bold)).foregroundStyle(Theme.ink)
                    .fixedSize(horizontal: false, vertical: true)
                progress(c)
                next(c)
                if c.needs == "owner" {
                    ReviewButton(number: c.number, sheet: $sheet, tab: $tab)
                }
                details(s, c)
            }
        }
        if let url = sessionLink(s.label) {
            Button {
                openURL(url)
            } label: {
                HStack(spacing: 8) {
                    Text("Open session in Claude").font(.body.weight(.semibold))
                    Image(systemName: "arrow.up.right").font(.footnote.weight(.bold))
                }
                .lineLimit(1)
                .foregroundStyle(Theme.background)
                .frame(maxWidth: .infinity, minHeight: 52)
                .background(RoundedRectangle(cornerRadius: 14).fill(Theme.ink))
            }
            .buttonStyle(.plain)
        }
    }

    private func progress(_ c: TrackedChange) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("PROGRESS").font(Theme.mono(11, .bold)).tracking(1)
                Spacer()
                Text("stop \(c.stop + 1) of 10").font(Theme.mono(12, .semibold))
            }
            .foregroundStyle(Theme.accent)
            ForEach(Array(railStops.enumerated()), id: \.offset) { item in
                stopRow(item.offset, item.element, c)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.accentSoft))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(railLabel(c))
    }

    private func stopRow(_ i: Int, _ name: String, _ c: TrackedChange) -> some View {
        HStack(spacing: 12) {
            stopDot(i, c)
            Text(i == c.stop ? c.stage : name)
                .font(.subheadline.weight(i == c.stop ? .bold : .regular))
                .foregroundStyle(i == c.stop ? Theme.ink : Theme.ink2)
            Spacer()
            if i == c.stop, let since = c.since {
                Text(elapsed(since: since, now: now))
                    .font(Theme.mono(12, .semibold)).foregroundStyle(Theme.accent)
            }
        }
        .frame(minHeight: 24)
    }

    @ViewBuilder
    private func stopDot(_ i: Int, _ c: TrackedChange) -> some View {
        if i < c.stop || (i == c.stop && c.mark == .done) {
            Circle().fill(Theme.ink).frame(width: 10, height: 10)
        } else if i == c.stop {
            Circle().fill(c.mark == .failed || c.mark == .stalled ? Theme.warn : Theme.accent)
                .frame(width: 10, height: 10)
                .overlay(Circle().stroke(Theme.card, lineWidth: 3).frame(width: 16, height: 16))
        } else {
            Circle().stroke(Theme.ink2, lineWidth: 1.5).frame(width: 10, height: 10)
        }
    }

    private func next(_ c: TrackedChange) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("NEXT · \(c.needs.uppercased())")
                .font(Theme.mono(11, .bold)).tracking(1).foregroundStyle(Theme.muted)
            Text(c.needs == "change" ? needsChip(c) : c.next)
                .font(.callout).foregroundStyle(Theme.ink)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 14).padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.background))
    }

    private func details(_ s: TrackedSession, _ c: TrackedChange) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
            GridRow {
                Text("Session").foregroundStyle(Theme.muted)
                HStack(spacing: 8) {
                    ModelBadge(model: s.model, small: true)
                    Text(s.role?.capitalized ?? "Session").foregroundStyle(Theme.ink)
                    ListeningDot(listening: s.listening)
                    Text(s.listening ? "listening" : "not listening")
                        .fontWeight(.semibold)
                        .foregroundStyle(s.listening ? Theme.good : Theme.muted)
                }
            }
            GridRow {
                Text("Brief").foregroundStyle(Theme.muted)
                Text(c.brief.map { "\($0.id) · \($0.title)" } ?? "none")
                    .foregroundStyle(Theme.ink)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.subheadline)
    }
}

/// A change's ten-stop rail.
struct Rail: View {
    let change: TrackedChange
    var height: CGFloat = 8

    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<10, id: \.self) { i in
                segment(i)
            }
        }
        .frame(height: height)
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
        case .stalled:
            RoundedRectangle(cornerRadius: 2).fill(Theme.chip)
                .overlay(Stripes().fill(Theme.warn).clipShape(RoundedRectangle(cornerRadius: 2)))
        case .failed:
            RoundedRectangle(cornerRadius: 2).fill(Theme.warn)
        case .done:
            RoundedRectangle(cornerRadius: 2).fill(Theme.ink)
        case .held:
            RoundedRectangle(cornerRadius: 2).fill(Theme.accentSoft)
                .overlay(Stripes().fill(Theme.accent).clipShape(RoundedRectangle(cornerRadius: 2)))
        }
    }
}

/// Diagonal stripes for a stalled or held stop.
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
