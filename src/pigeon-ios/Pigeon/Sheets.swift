// The cards that open over Home: one metric's chart by source and kind, an approval, Auto's
// length, and Profiles with Settings.

import PigeonCore
import SwiftUI

/// One metric over 1, 7 or 30 days, one source at a time, stacked by kind.
struct ChartSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let metric: String
    @State private var range = 1
    @State private var source = "cloudflare"
    @State private var stacks: Stacks?
    @State private var picked: Pick?

    /// A tapped segment: its bar and its kind.
    struct Pick: Equatable {
        let at: Int
        let kind: String
    }

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text(metric == "cost" ? "Cost" : "Errors").font(Theme.display(22))
                    Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark") }
                        .accessibilityLabel("Close")
                }
                Picker("Range", selection: $range) {
                    Text("1d").tag(1)
                    Text("7d").tag(7)
                    Text("30d").tag(30)
                }
                .pickerStyle(.segmented)
                Picker("Source", selection: $source) {
                    ForEach(sources, id: \.self) { Text(sourceName($0)).tag($0) }
                }
                .pickerStyle(.segmented)
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(stacks.map { formatted($0.total, metric: metric) } ?? "—")
                        .font(Theme.display(32))
                    Text(pickedText).font(.callout).foregroundStyle(Theme.ink2)
                }
                if metric == "cost" {
                    Text("Estimated from usage at published prices.")
                        .font(.caption).foregroundStyle(Theme.muted)
                }
                if let note = stacks?.note {
                    Label(note, systemImage: "info.circle")
                        .font(.caption).foregroundStyle(Theme.ink2)
                }
                bars.frame(height: 220).padding(14).block()
                legend
                Spacer()
            }
            .foregroundStyle(Theme.ink)
            .padding(20)
        }
        .task(id: "\(range)-\(source)") {
            picked = nil
            stacks = await store.stacks(metric: metric, source: source, range: range)
        }
        .presentationDetents([.large])
    }

    private var kinds: [String] { stacks?.legend.map(\.name) ?? [] }

    private func color(_ kind: String) -> Color {
        Theme.kind(kinds.firstIndex(of: kind) ?? 0)
    }

    private var pickedText: String {
        guard let p = picked, let s = stacks,
            let b = s.buckets.first(where: { $0.at == p.at }),
            let part = b.parts.first(where: { $0.name == p.kind })
        else { return "" }
        let now = Int(Date().timeIntervalSince1970)
        return "\(label(at: p.at, step: s.step, now: now)) · \(p.kind) \(formatted(part.value, metric: metric))"
    }

    private var bars: some View {
        GeometryReader { geo in
            let buckets = stacks?.buckets ?? []
            HStack(alignment: .bottom, spacing: 3) {
                ForEach(buckets, id: \.at) { b in
                    VStack(spacing: 2) {
                        Spacer(minLength: 0)
                        ForEach(
                            segments(of: b, among: buckets, height: geo.size.height - 4, minimum: 4)
                                .reversed(), id: \.name
                        ) { s in
                            let pick = Pick(at: b.at, kind: s.name)
                            let on = picked == nil || picked == pick
                            Button {
                                picked = picked == pick ? nil : pick
                            } label: {
                                Rectangle().fill(color(s.name)).opacity(on ? 1 : 0.35)
                                    .frame(height: s.height)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(
                                "\(label(at: b.at, step: stacks?.step ?? 3_600, now: Int(Date().timeIntervalSince1970))), \(s.name) \(formatted(s.value, metric: metric))"
                            )
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.ink).frame(height: 2) }
            .overlay {
                if (stacks?.buckets ?? []).allSatisfy({ $0.total == 0 }) {
                    Text(stacks == nil ? "No data yet" : "Nothing in this range")
                        .font(.footnote).foregroundStyle(Theme.muted)
                }
            }
        }
    }

    private var legend: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(stacks?.legend ?? [], id: \.name) { l in
                HStack(spacing: 10) {
                    RoundedRectangle(cornerRadius: 3).fill(color(l.name)).frame(width: 12, height: 12)
                    Text(l.name)
                    Spacer()
                    Text(formatted(l.total, metric: metric)).fontWeight(.semibold)
                }
                .font(.callout)
            }
        }
    }
}

/// One approval: what it is, its checks, and Face ID; then the merge's progress.
struct ApprovalSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let id: String
    @State private var detail: ApprovalDetail?
    @State private var failed: String?
    @State private var hold = false
    @State private var showChecks = false
    @State private var done = false

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if done {
                        approved
                    } else if let d = detail {
                        card(d)
                    } else if let failed {
                        Text(failed).foregroundStyle(Theme.accent)
                    } else {
                        ProgressView().frame(maxWidth: .infinity, minHeight: 200)
                    }
                }
                .foregroundStyle(Theme.ink)
                .padding(20)
            }
        }
        .task {
            do { detail = try await store.detail(id) } catch {
                failed = "This approval is no longer waiting."
            }
        }
        .presentationDetents([.large])
    }

    private func card(_ d: ApprovalDetail) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Text(verb(d.operation) + " · !\(d.number)")
                    .font(.footnote.weight(.semibold)).foregroundStyle(Theme.accent)
                Text(tierBadge(d.tier))
                    .font(Theme.mono(11, .semibold))
                    .foregroundStyle(d.tier == "owner" ? Theme.onAccent : Theme.ink2)
                    .padding(.horizontal, 8).padding(.vertical, 3)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(d.tier == "owner" ? Theme.accent : Theme.chip))
            }
            Text(d.title).font(Theme.display(22))
            VStack(alignment: .leading, spacing: 8) {
                row("Head", String((d.head ?? "none").prefix(7)), mono: true)
                row("Branch", d.branch, mono: true)
                row("Scope", "\(d.scope.count) path\(d.scope.count == 1 ? "" : "s")")
                Button {
                    showChecks.toggle()
                } label: {
                    HStack {
                        let passed = d.checks.filter { $0.status == "passed" }.count
                        Image(systemName: "checkmark.circle").foregroundStyle(Theme.good)
                        Text("\(passed) of \(d.checks.count) checks passed").fontWeight(.semibold)
                        Spacer()
                        Text(showChecks ? "Hide" : "Show").foregroundStyle(Theme.accent)
                    }
                }
                .buttonStyle(.plain)
                if showChecks {
                    ForEach(d.checks, id: \.name) { c in
                        Text("\(c.status == "passed" ? "✓" : "·") \(c.name)").font(Theme.mono(12))
                            .foregroundStyle(Theme.ink2)
                    }
                }
            }
            .font(.footnote)
            .padding(14)
            .block()
            if d.operation == "change.approve" {
                Toggle("Keep the merge for me", isOn: $hold).tint(Theme.accent)
            }
            if let e = store.error { Text(e).font(.footnote).foregroundStyle(Theme.accent) }
            Button {
                Task { done = await store.approve(id, hold: hold) }
            } label: {
                Label(
                    hold || d.operation != "change.approve"
                        ? "Approve with Face ID" : "Approve and merge with Face ID",
                    systemImage: "faceid"
                )
                .font(.headline)
                .frame(maxWidth: .infinity, minHeight: 52)
                .foregroundStyle(Theme.onAccent)
            }
            .background(RoundedRectangle(cornerRadius: 12).fill(Theme.accent))
            .background(RoundedRectangle(cornerRadius: 12).fill(Theme.shadow).offset(x: 3, y: 3))
            .disabled(store.busy)
        }
    }

    private var approved: some View {
        VStack(spacing: 14) {
            Image(systemName: "checkmark")
                .font(.system(size: 28, weight: .bold))
                .foregroundStyle(Theme.accent)
                .frame(width: 60, height: 60)
                .background(Circle().fill(Theme.accentSoft))
            Text("Approved · !\(detail?.number ?? 0)").font(Theme.display(22))
            Text(
                hold
                    ? "Approved. The merge waits for your second tap."
                    : "Your approval queued the merge. It lands once the merge gate passes."
            )
            .multilineTextAlignment(.center).foregroundStyle(Theme.ink2)
            Button("Done") { dismiss() }
                .font(.headline)
                .frame(maxWidth: .infinity, minHeight: 52)
                .block(radius: 12)
        }
        .padding(.top, 40)
    }

    private func row(_ k: String, _ v: String, mono: Bool = false) -> some View {
        HStack {
            Text(k).foregroundStyle(Theme.muted)
            Spacer()
            Text(v).font(mono ? Theme.mono(13) : .footnote).lineLimit(1)
        }
    }
}

/// Auto's length, then the owner's passkey.
struct AutoSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var hours = 4

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            VStack(alignment: .leading, spacing: 14) {
                Text("Auto approve for").font(Theme.display(22))
                HStack(spacing: 10) {
                    ForEach([1, 4, 8, 24], id: \.self) { h in
                        Button {
                            hours = h
                        } label: {
                            Text("\(h)h")
                                .font(Theme.mono(17, .bold))
                                .frame(maxWidth: .infinity, minHeight: 52)
                                .foregroundStyle(hours == h ? Theme.onAccent : Theme.ink)
                                .background(
                                    RoundedRectangle(cornerRadius: 12)
                                        .fill(hours == h ? Theme.accent : Theme.card))
                                .background(
                                    RoundedRectangle(cornerRadius: 12).fill(Theme.shadow)
                                        .offset(x: 3, y: 3))
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(hours == h ? .isSelected : [])
                    }
                }
                Text(
                    "Changes at the code tier and below are approved and merged as they pass. Anything touching secrets or destroying work still asks."
                )
                .font(.footnote).foregroundStyle(Theme.ink2)
                if let e = store.error { Text(e).font(.footnote).foregroundStyle(Theme.accent) }
                Button {
                    Task { if await store.startAuto(hours: hours) { dismiss() } }
                } label: {
                    Label("Start auto for \(hours == 1 ? "1 hour" : "\(hours) hours")", systemImage: "faceid")
                        .font(.headline)
                        .frame(maxWidth: .infinity, minHeight: 52)
                        .foregroundStyle(Theme.onAccent)
                }
                .background(RoundedRectangle(cornerRadius: 12).fill(Theme.accent))
                .background(RoundedRectangle(cornerRadius: 12).fill(Theme.shadow).offset(x: 3, y: 3))
                .disabled(store.busy)
                Spacer()
            }
            .foregroundStyle(Theme.ink)
            .padding(20)
        }
        .presentationDetents([.medium])
    }
}

/// Profiles: Platform alone in phase one, and Settings.
struct ProfilesSheet: View {
    @AppStorage("appearance") private var appearance = Appearance.system.rawValue
    @State private var settings = false

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            VStack(alignment: .leading, spacing: 14) {
                if settings {
                    Button {
                        settings = false
                    } label: {
                        Label("Profiles", systemImage: "chevron.left")
                    }
                    .foregroundStyle(Theme.accent)
                    Text("Settings").font(Theme.display(22))
                    Eyebrow(text: "Appearance")
                    HStack(spacing: 10) {
                        ForEach(Appearance.allCases) { a in
                            Button {
                                appearance = a.rawValue
                            } label: {
                                Text(a.label)
                                    .font(.subheadline.weight(.semibold))
                                    .frame(maxWidth: .infinity, minHeight: 48)
                                    .foregroundStyle(
                                        appearance == a.rawValue ? Theme.onAccent : Theme.ink)
                                    .background(
                                        RoundedRectangle(cornerRadius: 12)
                                            .fill(appearance == a.rawValue ? Theme.accent : Theme.card))
                                    .background(
                                        RoundedRectangle(cornerRadius: 12).fill(Theme.shadow)
                                            .offset(x: 3, y: 3))
                            }
                            .buttonStyle(.plain)
                            .accessibilityAddTraits(appearance == a.rawValue ? .isSelected : [])
                        }
                    }
                } else {
                    Text("Profiles").font(Theme.display(22))
                    HStack(spacing: 12) {
                        Image("Avatar").resizable().frame(width: 44, height: 44).clipShape(Circle())
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Platform").font(.headline)
                            Text(API.host).font(.footnote).foregroundStyle(Theme.muted)
                        }
                        Spacer()
                        Image(systemName: "checkmark").foregroundStyle(Theme.accent)
                    }
                    .padding(12)
                    .block(shadow: Theme.accent)
                    Button {
                        settings = true
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "gearshape")
                            Text("Settings").font(.headline)
                            Spacer()
                            Image(systemName: "chevron.right").foregroundStyle(Theme.muted)
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 52)
                        .block()
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
            .foregroundStyle(Theme.ink)
            .padding(20)
        }
        .presentationDetents([.medium])
    }
}
