// Now: Auto, the active sessions and what needs the owner as two cards, and the merge queue in
// order, each opening a sheet; live through /v0/app/events. See
// .claude/decisions/2026-11-15-runtime-ios-app-now-tab.md.

import PigeonCore
import SwiftUI

/// The change whose sheet is open.
struct OpenedChange: Identifiable {
    let id: Int
}

/// The approval whose card is open over a sheet.
struct OpenedApproval: Identifiable {
    let id: String
}

/// The list a card opens.
enum NowPanel: String, Identifiable {
    case sessions, needs
    var id: String { rawValue }
}

/// A model class's colour, shared by the chips, dots and badges.
func classColor(_ c: ModelClass) -> Color {
    switch c {
    case .opus: return Theme.accent
    case .sonnet: return Theme.color(0x3A78B5, 0x4C86C6)
    case .haiku: return Theme.color(0x2D8C6A, 0x2F9C78)
    case .fable: return Theme.color(0x8A6420, 0xC79A3A)
    case .other: return Theme.muted
    }
}

struct NowView: View {
    @EnvironmentObject private var store: Store
    @Binding var sheet: Sheet?
    @State private var panel: NowPanel?
    @State private var opened: OpenedChange?
    @State private var filter: TrackerFilter = .all
    @State private var showSilent = false
    @State private var showAll = false
    @State private var now = Int(Date().timeIntervalSince1970)

    private let tick = Timer.publish(every: 30, on: .main, in: .common).autoconnect()
    private let shownCells = 10

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    TopBar(sheet: $sheet)
                    if let error = store.error {
                        Text(error).font(.footnote).foregroundStyle(Theme.accent)
                    }
                    Text("Now").font(Theme.display(22)).foregroundStyle(Theme.ink)
                        .padding(.top, 8)
                    if let auto = store.home?.auto {
                        AutoBanner(window: auto, now: now)
                    } else if store.home != nil {
                        AutoOff()
                    }
                    cards
                    queue
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 32)
            }
            .refreshable {
                await store.refresh()
                await store.loadTracker()
            }
        }
        .task { await store.follow() }
        .onReceive(tick) { _ in now = Int(Date().timeIntervalSince1970) }
        .sheet(item: $panel) { p in
            Group {
                switch p {
                case .sessions:
                    SessionsSheet(filter: $filter, showSilent: $showSilent, now: now)
                case .needs:
                    NeedsMeSheet(now: now)
                }
            }
            .environmentObject(store)
        }
        .sheet(item: $opened) { o in
            ChangeSheet(number: o.id, now: now).environmentObject(store)
        }
    }

    private var sessions: [TrackedSession] { store.tracker?.sessions ?? [] }
    private var needs: [NeedItem] { needsMe(home: store.home, tracker: store.tracker) }

    private var cards: some View {
        Grid(horizontalSpacing: 14) {
            GridRow {
                Button {
                    panel = .sessions
                } label: {
                    VStack(alignment: .leading, spacing: 10) {
                        Eyebrow(text: "Active sessions")
                        Text("\(sessions.count)").font(Theme.display(44)).foregroundStyle(Theme.ink)
                        FlowRow(spacing: 6) {
                            ForEach(classTally(sessions)) { entry in
                                ClassChip(model: entry.model, count: entry.count)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 148, alignment: .topLeading)
                    .padding(14)
                    .block()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(
                    "\(sessions.count) active sessions: "
                        + classTally(sessions).map { "\($0.count) \($0.model.label)" }.joined(separator: ", "))
                .accessibilityHint("Opens the list")

                Button {
                    panel = .needs
                } label: {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("NEEDS ME").font(Theme.mono(12, .semibold)).tracking(1)
                            .foregroundStyle(needs.isEmpty ? Theme.muted : Theme.accentSoft)
                        Text("\(needs.count)").font(Theme.display(44))
                            .foregroundStyle(needs.isEmpty ? Theme.ink : Theme.onAccent)
                        Text(needsSummary(needs)).font(.footnote)
                            .foregroundStyle(needs.isEmpty ? Theme.muted : Theme.accentSoft)
                            .multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: .infinity, minHeight: 148, alignment: .topLeading)
                    .padding(14)
                    .background(
                        RoundedRectangle(cornerRadius: 14).fill(needs.isEmpty ? Theme.card : Theme.accent))
                    .background(RoundedRectangle(cornerRadius: 14).fill(Theme.shadow).offset(x: 3, y: 3))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(needs.count) need you: \(needsSummary(needs))")
                .accessibilityHint("Opens the list")
            }
        }
    }

    @ViewBuilder
    private var queue: some View {
        let cells = store.tracker.map(mergeQueue) ?? []
        let queued = cells.filter { $0.position > 0 }.count
        HStack(alignment: .firstTextBaseline) {
            Text("Merge queue").font(.headline).foregroundStyle(Theme.ink)
            Spacer()
            Text(
                (cells.first?.position == 0 ? "1 merging · " : "") + "\(queued) queued"
            )
            .font(Theme.mono(12)).foregroundStyle(Theme.muted)
        }
        .padding(.top, 10)
        if cells.isEmpty {
            Text("Nothing queued.").font(.callout).foregroundStyle(Theme.muted)
        }
        ForEach(showAll ? cells : Array(cells.prefix(shownCells))) { q in
            QueueCell(cell: q) { opened = OpenedChange(id: q.number) }
        }
        if !showAll, cells.count > shownCells {
            Button {
                showAll = true
            } label: {
                Text("Show \(cells.count - shownCells) more")
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.accent)
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(.plain)
        }
    }
}

/// A class and its count, as a chip with its colour's dot.
struct ClassChip: View {
    let model: ModelClass
    let count: Int

    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(classColor(model)).frame(width: 8, height: 8)
            Text("\(count) \(model.label)").font(Theme.mono(12, .semibold))
        }
        .foregroundStyle(Theme.ink2)
        .padding(.horizontal, 8).frame(minHeight: 24)
        .background(Capsule().fill(Theme.chip))
    }
}

/// One change in the merge queue: its position, number, class, session, subject and wait.
struct QueueCell: View {
    let cell: QueuedChange
    let open: () -> Void

    private var meta: String {
        let head = cell.position == 0 ? "MERGING · " : ""
        let session = cell.session.map { " · " + shortLabel($0) } ?? ""
        return head + "!\(cell.number) · " + ModelClass(cell.model).label.uppercased() + session
    }

    private var spoken: String {
        let place = cell.position == 0 ? "Merging" : "Position \(cell.position)"
        let subject = scopeAndSubject(cell.title).subject
        return place + ", change \(cell.number), " + ModelClass(cell.model).label + ": " + subject + ". " + queueWait(cell)
    }

    var body: some View {
        let merging = cell.position == 0
        let model = ModelClass(cell.model)
        Button(action: open) {
            HStack(spacing: 12) {
                Group {
                    if merging {
                        Image(systemName: "arrow.triangle.merge").font(.body.weight(.bold))
                            .foregroundStyle(Theme.onAccent)
                    } else {
                        Text("\(cell.position)").font(Theme.mono(15, .bold)).foregroundStyle(Theme.ink)
                    }
                }
                .frame(width: 34, height: 34)
                .background(RoundedRectangle(cornerRadius: 10).fill(merging ? Theme.good : Theme.chip))
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Circle().fill(classColor(model)).frame(width: 8, height: 8)
                        Text(meta)
                            .font(Theme.mono(12, .semibold))
                            .foregroundStyle(merging ? Theme.good : Theme.ink2)
                    }
                    Text(scopeAndSubject(cell.title).subject)
                        .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
                        .lineLimit(merging ? 2 : 1)
                        .multilineTextAlignment(.leading)
                    Text(queueWait(cell)).font(.caption)
                        .foregroundStyle(cell.needs == "owner" ? Theme.accent : Theme.muted)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted)
            }
            .padding(.horizontal, 14).padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .block(shadow: merging ? Theme.good : (cell.needs == "owner" ? Theme.accent : Theme.shadow))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(spoken)
        .accessibilityHint("Opens the change")
    }
}

/// The Active sessions sheet: counts, the tracker's filters and silent toggle, and a card per
/// session.
struct SessionsSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @Binding var filter: TrackerFilter
    @Binding var showSilent: Bool
    let now: Int
    @State private var opened: OpenedChange?

    var body: some View {
        let tracker = store.tracker ?? Tracker(sessions: [], counts: TrackerCounts(owner: 0, stalled: 0, provider: 0))
        let active = effectiveFilter(tracker, filter)
        let shown = visible(tracker, by: filter, showSilent: showSilent)
        let silent = silentCount(tracker, by: filter)
        ZStack {
            Theme.background.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Active sessions").font(Theme.display(22)).foregroundStyle(Theme.ink)
                        Spacer()
                        Button("Done") { dismiss() }
                            .font(.body.weight(.semibold)).foregroundStyle(Theme.accent)
                            .frame(minHeight: 44)
                    }
                    Text(countsLine(tracker))
                        .font(Theme.mono(12)).foregroundStyle(Theme.muted)
                    FlowRow {
                        chip("All", .all, active)
                        if tracker.counts.owner > 0 { chip("\(tracker.counts.owner) need you", .owner, active) }
                        if tracker.counts.stalled > 0 { chip("\(tracker.counts.stalled) stalled", .stalled, active) }
                        if tracker.counts.provider > 0 {
                            chip("\(tracker.counts.provider) on a provider", .provider, active)
                        }
                        if silent > 0 {
                            Button {
                                showSilent.toggle()
                            } label: {
                                chipLabel(
                                    Text(
                                        "\(Image(systemName: showSilent ? "circle.inset.filled" : "circle.dashed")) \(silent) silent"
                                    ), on: showSilent)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("\(silent) silent sessions")
                            .accessibilityValue(showSilent ? "shown" : "hidden")
                        }
                    }
                    ForEach(shown) { s in
                        SessionSummaryCard(session: s) { opened = OpenedChange(id: $0) }
                    }
                    if shown.isEmpty {
                        Text("Nothing running.").font(.callout).foregroundStyle(Theme.muted)
                    }
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
                .padding(20)
            }
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .sheet(item: $opened) { o in
            ChangeSheet(number: o.id, now: now).environmentObject(store)
        }
    }

    private func countsLine(_ t: Tracker) -> String {
        let classes = classTally(t.sessions).map { "\($0.count) \($0.model.label)" }
        let changes = t.sessions.reduce(0) { $0 + $1.changes.count }
        return (["\(t.sessions.count) active"] + classes + ["\(changes) changes"]).joined(separator: " · ")
    }

    private func chip(_ text: String, _ f: TrackerFilter, _ active: TrackerFilter) -> some View {
        let on = active == f
        return Button {
            filter = on && f != .all ? .all : f
        } label: {
            chipLabel(Text(text), on: on)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    private func chipLabel(_ text: Text, on: Bool) -> some View {
        text
            .font(.footnote.weight(.semibold))
            .foregroundStyle(on ? Theme.onAccent : Theme.ink2)
            .padding(.horizontal, 10).frame(minHeight: 32)
            .background(Capsule().fill(on ? Theme.accent : Theme.chip))
    }
}

/// A session in the Active sessions sheet: class and role, its title, what it's doing or needs,
/// its changes, and a link into Claude.
struct SessionSummaryCard: View {
    @Environment(\.openURL) private var openURL
    let session: TrackedSession
    let open: (Int) -> Void

    var body: some View {
        let quiet = isSilent(session)
        let needsOwner = session.changes.contains { $0.needs == "owner" }
        let failing = session.changes.contains { $0.mark == .failed || $0.mark == .stalled }
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                ModelBadge(model: session.model, small: true)
                Text(session.role ?? "session").font(Theme.mono(12)).foregroundStyle(Theme.muted)
                ListeningDot(listening: session.listening)
                Spacer()
                if let url = sessionLink(session.label) {
                    Button {
                        openURL(url)
                    } label: {
                        HStack(spacing: 4) {
                            Text("Open in Claude")
                            Image(systemName: "arrow.up.right").font(.caption.weight(.bold))
                        }
                        .font(.footnote.weight(.semibold)).foregroundStyle(Theme.accent)
                        .frame(minHeight: 32)
                    }
                    .buttonStyle(.plain)
                }
            }
            Text(sessionTitle(session))
                .font(.callout.weight(.semibold)).foregroundStyle(Theme.ink)
                .fixedSize(horizontal: false, vertical: true)
            Text(doingLine(session))
                .font(.footnote.weight(.medium))
                .foregroundStyle(needsOwner ? Theme.accent : (failing ? Theme.warn : Theme.ink2))
                .fixedSize(horizontal: false, vertical: true)
            if !session.changes.isEmpty {
                FlowRow(spacing: 6) {
                    ForEach(session.changes) { c in
                        Button {
                            open(c.number)
                        } label: {
                            Text(changeChip(c))
                                .font(Theme.mono(12, .semibold)).foregroundStyle(Theme.ink2)
                                .padding(.horizontal, 8).frame(minHeight: 28)
                                .background(RoundedRectangle(cornerRadius: 8).fill(Theme.chip))
                        }
                        .buttonStyle(.plain)
                        .accessibilityHint("Opens the change")
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .modifier(CardStyle(quiet: quiet, shadow: needsOwner ? Theme.accent : (failing ? Theme.warn : Theme.shadow)))
    }

    private func changeChip(_ c: TrackedChange) -> String {
        if let p = queuePosition(c.stage) { return p == 0 ? "!\(c.number) merging" : "!\(c.number) #\(p)" }
        return "!\(c.number) \(c.stage)"
    }
}

/// The Needs me sheet: the owner's items, approvals and changes at `needs: owner`, each answered
/// in place.
struct NeedsMeSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let now: Int
    @State private var approval: OpenedApproval?
    @State private var opened: OpenedChange?

    var body: some View {
        let items = needsMe(home: store.home, tracker: store.tracker)
        ZStack {
            Theme.background.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Needs me").font(Theme.display(22)).foregroundStyle(Theme.ink)
                        Spacer()
                        Button("Done") { dismiss() }
                            .font(.body.weight(.semibold)).foregroundStyle(Theme.accent)
                            .frame(minHeight: 44)
                    }
                    Text(items.isEmpty ? "Nothing waiting" : "\(items.count) waiting")
                        .font(Theme.mono(12)).foregroundStyle(Theme.muted)
                    ForEach(items) { item in
                        row(item)
                    }
                    if !items.isEmpty {
                        Text("Answered items leave this list; approvals stay listed until they no longer apply.")
                            .font(.footnote).foregroundStyle(Theme.muted)
                            .frame(maxWidth: .infinity)
                            .multilineTextAlignment(.center)
                    }
                }
                .padding(20)
            }
            .refreshable { await store.refresh() }
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .sheet(item: $approval) { a in
            ApprovalSheet(id: a.id).environmentObject(store)
        }
        .sheet(item: $opened) { o in
            ChangeSheet(number: o.id, now: now).environmentObject(store)
        }
    }

    @ViewBuilder
    private func row(_ item: NeedItem) -> some View {
        switch item {
        case .item(let o):
            ItemCard(item: o)
        case .approval(let a):
            NeedCard(
                eyebrow: "Approval · \(verb(a.operation)) · !\(a.number)", title: a.title, detail: nil,
                action: "Review and approve", run: { approval = OpenedApproval(id: a.id) },
                details: { opened = OpenedChange(id: a.number) })
        case .change(let c, _):
            NeedCard(
                eyebrow: "Needs you · !\(c.number)", title: scopeAndSubject(c.title).subject,
                detail: c.next, action: "Open the change", run: { opened = OpenedChange(id: c.number) },
                details: nil)
        }
    }
}

/// A card for something waiting on the owner, with its one action.
struct NeedCard: View {
    let eyebrow: String
    let title: String
    let detail: String?
    let action: String
    let run: () -> Void
    let details: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(eyebrow).font(.footnote.weight(.semibold)).foregroundStyle(Theme.accent)
            Text(title).font(.callout.weight(.semibold)).foregroundStyle(Theme.ink)
                .fixedSize(horizontal: false, vertical: true)
            if let detail {
                Text(detail).font(.footnote).foregroundStyle(Theme.ink2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 12) {
                Button(action: run) {
                    Text(action).font(.subheadline.weight(.semibold)).foregroundStyle(Theme.onAccent)
                        .padding(.horizontal, 14).frame(minHeight: 40)
                        .background(RoundedRectangle(cornerRadius: 10).fill(Theme.accent))
                }
                .buttonStyle(.plain)
                if let details {
                    Button("Details", action: details)
                        .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.accent)
                        .frame(minHeight: 40)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .block(shadow: Theme.accent)
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

/// The session's class as a badge, in its colour.
struct ModelBadge: View {
    let model: String?
    var small = false

    var body: some View {
        let c = ModelClass(model)
        Text(c.label.uppercased())
            .font(Theme.mono(small ? 10 : 11, .semibold))
            .foregroundStyle(c == .other ? Theme.ink : Theme.onAccent)
            .padding(.horizontal, small ? 6 : 8).padding(.vertical, small ? 2 : 3)
            .background(RoundedRectangle(cornerRadius: 5).fill(c == .other ? Theme.chip : classColor(c)))
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


/// Opens the change's listed approval; until it is listed, says so.
struct ReviewButton: View {
    @EnvironmentObject private var store: Store
    let number: Int
    var listed: String?
    let open: (String) -> Void

    var body: some View {
        let id = listed ?? store.home?.approvals.first(where: { $0.number == number })?.id
        Button {
            if let id { open(id) }
        } label: {
            Text(id == nil ? "Its approval isn't listed yet" : "Approve with passkey")
                .font(.callout.weight(.semibold))
                .foregroundStyle(Theme.onAccent)
                .padding(.horizontal, 18).frame(maxWidth: .infinity, minHeight: 44)
                .background(Capsule().fill(id == nil ? Theme.muted : Theme.accent))
        }
        .buttonStyle(.plain)
        .disabled(id == nil)
    }
}

/// A change's sheet: its title, place in the queue, progress, what's next and what it waits on,
/// its checks, its session, and a link into Claude; the change route fills in the checks.
struct ChangeSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    let now: Int
    @State private var number: Int
    @State private var detail: ChangeDetail?
    @State private var approval: OpenedApproval?

    init(number: Int, now: Int) {
        self.now = now
        _number = State(initialValue: number)
    }

    private var found: (session: TrackedSession, change: TrackedChange)? {
        for s in store.tracker?.sessions ?? [] {
            if let c = s.changes.first(where: { $0.number == number }) { return (s, c) }
        }
        return nil
    }

    private var queued: QueuedChange? {
        store.tracker.map(mergeQueue)?.first { $0.number == number }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Text(header).font(Theme.mono(13, .semibold)).foregroundStyle(Theme.muted)
                if let tag = scopeAndSubject(title).tag {
                    ScopeTag(text: tag)
                }
                Spacer()
                Button("Done") { dismiss() }
                    .font(.body.weight(.semibold)).foregroundStyle(Theme.accent)
                    .frame(minHeight: 44)
            }
            if title.isEmpty {
                Text("This change has left the tracker.")
                    .font(.callout).foregroundStyle(Theme.muted)
                Spacer()
            } else {
                content
            }
        }
        .padding(.horizontal, 20).padding(.top, 16).padding(.bottom, 20)
        .background(Theme.card)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .task(id: number) { detail = await store.changeDetail(number) }
        .sheet(item: $approval) { a in
            ApprovalSheet(id: a.id).environmentObject(store)
        }
    }

    private var title: String { found?.change.title ?? queued?.title ?? detail?.title ?? "" }
    private var stage: String { found?.change.stage ?? queued?.stage ?? detail?.stage ?? "" }
    private var needs: String { found?.change.needs ?? queued?.needs ?? detail?.needs ?? "none" }
    private var nextLine: String { found?.change.next ?? queued?.next ?? detail?.next ?? "" }
    private var waitsOn: Int? { found?.change.waitsOn ?? queued?.waitsOn ?? detail?.waitsOn }
    private var label: String? { found?.session.label ?? queued?.session ?? detail?.session }
    private var model: String? { found?.session.model ?? queued?.model ?? detail?.model }

    private var header: String {
        var parts = ["!\(number)"]
        if let p = queuePosition(stage) {
            parts.append(p == 0 ? "MERGING" : "QUEUED \(p) OF \(stage.split(separator: "/").last ?? "")")
        } else if !stage.isEmpty {
            parts.append(stage.uppercased())
        }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(scopeAndSubject(title).subject)
                    .font(.title3.weight(.bold)).foregroundStyle(Theme.ink)
                    .fixedSize(horizontal: false, vertical: true)
                if needs == "owner" {
                    ReviewButton(number: number, listed: detail?.approval) {
                        approval = OpenedApproval(id: $0)
                    }
                }
                next
                if let c = found?.change {
                    progress(c)
                }
                checks
                session
            }
        }
        if let label, let url = sessionLink(label) {
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

    private var next: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("NEXT · \(needs.uppercased())")
                .font(Theme.mono(11, .bold)).tracking(1).foregroundStyle(Theme.muted)
            Text(nextLine)
                .font(.callout).foregroundStyle(Theme.ink)
                .fixedSize(horizontal: false, vertical: true)
            if let w = waitsOn {
                Button {
                    number = w
                } label: {
                    HStack(spacing: 4) {
                        Text("Waits on !\(w)")
                        Image(systemName: "chevron.right").font(.caption.weight(.bold))
                    }
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.accent)
                    .frame(minHeight: 36)
                }
                .buttonStyle(.plain)
            }
            if let since = found?.change.since ?? queued?.since ?? detail?.since {
                Text("In this stage \(elapsed(since: since, now: now))")
                    .font(Theme.mono(12)).foregroundStyle(Theme.muted)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 14).padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.background))
    }

    @ViewBuilder
    private var checks: some View {
        if let d = detail, !d.jobs.isEmpty || d.github.contains(where: { $0.run != nil }) {
            VStack(alignment: .leading, spacing: 0) {
                Text("CHECKS").font(Theme.mono(11, .bold)).tracking(1).foregroundStyle(Theme.muted)
                    .padding(.bottom, 6)
                ForEach(d.jobs, id: \.name) { j in
                    HStack(spacing: 10) {
                        Image(systemName: jobGlyph(j.status)).foregroundStyle(jobColor(j.status))
                        Text(j.name).font(Theme.mono(14)).foregroundStyle(Theme.ink)
                        Spacer()
                        Text(j.status).font(.footnote.weight(.semibold)).foregroundStyle(jobColor(j.status))
                    }
                    .frame(minHeight: 36)
                    .accessibilityElement(children: .combine)
                }
                ForEach(Array(d.github.enumerated()), id: \.offset) { item in
                    if let run = item.element.run, let url = URL(string: run) {
                        Button {
                            openURL(url)
                        } label: {
                            HStack(spacing: 4) {
                                Text(item.element.waiting.map { "GitHub run · \($0)" } ?? "GitHub run")
                                Image(systemName: "arrow.up.right").font(.caption.weight(.bold))
                            }
                            .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.accent)
                            .frame(minHeight: 36)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(.horizontal, 14).padding(.vertical, 12)
            .background(RoundedRectangle(cornerRadius: 14).fill(Theme.background))
        }
    }

    private var session: some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
            GridRow {
                Text("Session").foregroundStyle(Theme.muted)
                HStack(spacing: 8) {
                    ModelBadge(model: model, small: true)
                    Text(found?.session.role?.capitalized ?? label.map(shortLabel) ?? "none")
                        .foregroundStyle(Theme.ink)
                    if let s = found?.session {
                        ListeningDot(listening: s.listening)
                        Text(s.listening ? "listening" : "not listening")
                            .fontWeight(.semibold)
                            .foregroundStyle(s.listening ? Theme.good : Theme.muted)
                    }
                }
            }
            if let b = found?.change.brief {
                GridRow {
                    Text("Brief").foregroundStyle(Theme.muted)
                    Text("\(b.id) · \(b.title)")
                        .foregroundStyle(Theme.ink)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .font(.subheadline)
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
                HStack(spacing: 12) {
                    stopDot(item.offset, c)
                    Text(item.offset == c.stop ? c.stage : item.element)
                        .font(.subheadline.weight(item.offset == c.stop ? .bold : .regular))
                        .foregroundStyle(item.offset == c.stop ? Theme.ink : Theme.ink2)
                    Spacer()
                }
                .frame(minHeight: 24)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.accentSoft))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(railLabel(c))
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

    private func jobGlyph(_ status: String) -> String {
        switch status {
        case "passed": return "checkmark.circle.fill"
        case "failed", "cancelled": return "xmark.circle.fill"
        default: return "circle.dotted"
        }
    }

    private func jobColor(_ status: String) -> Color {
        switch status {
        case "passed": return Theme.good
        case "failed", "cancelled": return Theme.warn
        default: return Theme.muted
        }
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
