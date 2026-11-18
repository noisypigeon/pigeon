// The cards that open over Home: one metric's chart by source and kind, an approval, and
// Instances.

import PigeonCore
import UIKit
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

/// Instances: the platform, then each personal instance this account administers, from
/// `GET /v0/app/instances`; see the accounts, profiles and instances ADR §4 and the learner ADR §2.
struct InstancesSheet: View {
    @EnvironmentObject private var store: Store
    @State private var open: InstanceEntry?
    @State private var adding = false

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text("Instances").font(Theme.display(22))
                    ForEach(store.instances.isEmpty ? [platform] : store.instances, id: \.key) { i in
                        Button { if i.learner && i.ready { open = i } } label: { row(i) }
                            .buttonStyle(.plain)
                    }
                    Button { adding = true } label: {
                        Label("Add a practice instance", systemImage: "plus")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(Theme.accent)
                    }
                    Spacer()
                }
                .foregroundStyle(Theme.ink)
                .padding(20)
            }
        }
        .presentationDetents([.medium, .large])
        .task { await store.loadInstances() }
        .fullScreenCover(item: $open) { LearnerAdminView(instance: $0) }
        .sheet(isPresented: $adding) { StandUpSheet() }
    }

    private var platform: InstanceEntry {
        InstanceEntry(
            id: "platform", name: "Platform", mode: "platform", shape: "full",
            base_url: "https://\(API.host)", accent: nil, features: [], status: "ready", why: nil)
    }

    private func row(_ i: InstanceEntry) -> some View {
        HStack(spacing: 12) {
            if i.mode == "platform" {
                Image("Avatar").resizable().frame(width: 44, height: 44).clipShape(Circle())
            } else {
                Image(systemName: "graduationcap")
                    .frame(width: 44, height: 44)
                    .background(Circle().fill(Theme.accent.opacity(0.15)))
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(i.name).font(.headline)
                Text(subtitle(i)).font(.footnote).foregroundStyle(Theme.muted)
            }
            Spacer()
            if i.mode == "platform" {
                Image(systemName: "checkmark").foregroundStyle(Theme.accent)
            } else if i.ready {
                Image(systemName: "chevron.right").foregroundStyle(Theme.muted)
            }
        }
        .padding(12)
        .block(shadow: Theme.accent)
    }

    private func subtitle(_ i: InstanceEntry) -> String {
        switch i.status ?? "ready" {
        case "standing-up": return "Standing up… pull to refresh"
        case "failed": return "Stand-up failed: \(i.why ?? "unknown"). Add it again to resume."
        default: return i.host ?? (i.learner ? "Practice and progress" : (URL(string: i.base_url ?? "")?.host ?? ""))
        }
    }
}

/// Stands up a personal practice instance: a name, and a one-time token for its Cloudflare
/// account, pasted once and proved with the owner's passkey.
struct StandUpSheet: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var label = ""
    @State private var token = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name, seen only by you", text: $name)
                } footer: {
                    Text("Kept only on your platform's private list, never in the repository.")
                }
                Section {
                    HStack(spacing: 0) {
                        TextField("label", text: $label)
                            .textInputAutocapitalization(.never).autocorrectionDisabled()
                        Text(".\(standaloneZone)").foregroundStyle(Theme.muted)
                    }
                } footer: {
                    Text("Its address and identifier: 3 to 32 lowercase letters, digits or dashes. Kept only on your platform, never in the repository.")
                }
                Section {
                    Button("Paste account token") { token = UIPasteboard.general.string ?? "" }
                    if !token.isEmpty { Text("Token pasted").foregroundStyle(Theme.muted) }
                } footer: {
                    Text("From the new account's API Tokens page: Workers Scripts Edit, Workers R2 Storage Edit, and Account Settings Read. Revoke it once the instance is ready.")
                }
                if let e = store.error { Text(e).foregroundStyle(Theme.kind(1)) }
                Button("Stand it up") {
                    Task {
                        if await store.standUp(name: name, label: label, token: token) {
                            UIPasteboard.general.string = ""
                            token = ""
                            dismiss()
                        }
                    }
                }
                .disabled(name.isEmpty || !isLabel(label) || token.count < 20 || store.busy)
            }
            .navigationTitle("Practice instance")
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
    }
}
