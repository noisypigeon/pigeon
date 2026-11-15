// Insights: /v0/app/ops beside cost and errors; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md's 2026-11-15 amendment.

import PigeonCore
import SwiftUI

struct InsightsView: View {
    @EnvironmentObject private var store: Store
    @Binding var sheet: Sheet?
    @State private var range = 1

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    TopBar(sheet: $sheet)
                    if let error = store.error {
                        Text(error).font(.footnote).foregroundStyle(Theme.accent)
                    }
                    Text("Insights").font(Theme.display(22)).foregroundStyle(Theme.ink)
                        .padding(.top, 8)
                    Picker("Range", selection: $range) {
                        Text("Today").tag(1)
                        Text("7 days").tag(7)
                        Text("30 days").tag(30)
                    }
                    .pickerStyle(.segmented)
                    if let ops = store.ops {
                        grid(ops)
                        pipeline(ops)
                        queue(ops)
                        providers(ops)
                        throughput(ops)
                        jobs(ops)
                    }
                    HStack(spacing: 10) {
                        tile("Cost", metric: "cost", stacks: store.home?.insights.cost)
                        tile("Errors", metric: "errors", stacks: store.home?.insights.errors)
                    }
                    if let note = store.ops?.note {
                        Text(note).font(.footnote).foregroundStyle(Theme.muted)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 32)
            }
            .refreshable {
                await store.loadOps(range: range)
                await store.refresh()
            }
        }
        .task(id: range) { await store.loadOps(range: range) }
    }

    // MARK: tiles

    private func grid(_ ops: Ops) -> some View {
        let columns = [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)]
        return LazyVGrid(columns: columns, spacing: 10) {
            stat("Deploys", "\(ops.deploys.passed) / \(ops.deploys.total)", "passed / total")
            stat("Merge queue", "\(ops.queue.depth)", "p50 wait \(minutesText(ops.queue.p50))")
            stat(
                "Approval wait", minutesText(ops.approvals.p50),
                "p95 \(minutesText(ops.approvals.p95))")
            stat(
                "Failed jobs", "\(ops.jobs.count)", "in range",
                tint: ops.jobs.count > 0 ? Theme.warn : Theme.ink)
        }
    }

    private func stat(_ title: String, _ value: String, _ note: String, tint: Color = Theme.ink)
        -> some View
    {
        VStack(alignment: .leading, spacing: 4) {
            Eyebrow(text: title)
            Text(value).font(Theme.mono(22, .bold)).foregroundStyle(tint)
            Text(note).font(Theme.mono(11)).foregroundStyle(Theme.muted)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .block()
        .accessibilityElement(children: .combine)
    }

    private func card<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content)
        -> some View
    {
        VStack(alignment: .leading, spacing: 10) {
            Eyebrow(text: title)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .block()
    }

    private func time(_ t: Int) -> String {
        Date(timeIntervalSince1970: TimeInterval(t))
            .formatted(date: .abbreviated, time: .shortened)
    }

    // MARK: cards

    private func pipeline(_ ops: Ops) -> some View {
        card("Pipeline") {
            ForEach(ops.deploys.recent) { d in
                let passed = d.outcome == "passed"
                HStack(spacing: 8) {
                    Image(systemName: passed ? "checkmark.circle.fill" : "xmark.square.fill")
                        .foregroundStyle(passed ? Theme.good : Theme.warn)
                    Text(String(d.sha.prefix(7))).font(Theme.mono(12, .semibold))
                    Spacer()
                    Text(time(d.at)).font(Theme.mono(12)).foregroundStyle(Theme.muted)
                }
                .accessibilityElement(children: .combine)
            }
            if ops.deploys.recent.isEmpty {
                Text("No deploys.").font(.callout).foregroundStyle(Theme.muted)
            }
        }
    }

    private func queue(_ ops: Ops) -> some View {
        card("Merge queue") {
            bars(barHeights(ops.queue.waits), accentLast: false)
            Text(
                "\(ops.queue.waits.count) waits · p50 \(minutesText(ops.queue.p50)) · max \(minutesText(ops.queue.max))"
            )
            .font(Theme.mono(12)).foregroundStyle(Theme.muted)
        }
    }

    private func providers(_ ops: Ops) -> some View {
        card("CI providers") {
            ForEach(ops.providers) { p in
                HStack(spacing: 8) {
                    Rectangle().fill(Theme.source(p.provider)).frame(width: 10, height: 10)
                    Text(sourceName(p.provider)).font(.subheadline.weight(.semibold))
                    Text(p.health).font(Theme.mono(12)).foregroundStyle(Theme.muted)
                    Spacer()
                    Text(
                        p.cap.map { "\(Int(p.minutes)) / \(Int($0)) min" } ?? "\(Int(p.minutes)) min"
                    )
                    .font(Theme.mono(12))
                }
                .accessibilityElement(children: .combine)
            }
        }
    }

    private func throughput(_ ops: Ops) -> some View {
        let t = ops.throughput
        return card("Throughput") {
            bars(barHeights(t.days.map(\.merged)), accentLast: true)
            Text(
                "\(t.days.reduce(0) { $0 + $1.merged }) merged · lead p50 \(minutesText(t.leadP50))"
            )
            .font(Theme.mono(12)).foregroundStyle(Theme.muted)
            Text(
                "owner \(t.by.owner) · window \(t.by.window) · mandate \(t.by.mandate) · policy \(t.by.policy)"
            )
            .font(Theme.mono(12)).foregroundStyle(Theme.muted)
        }
    }

    private func jobs(_ ops: Ops) -> some View {
        card("Failed jobs") {
            ForEach(Array(ops.jobs.enumerated()), id: \.offset) { _, j in
                HStack(spacing: 8) {
                    Text(j.name).font(Theme.mono(12, .semibold))
                    if let c = j.change { Text("!\(c)").font(Theme.mono(12)) }
                    Text(j.status).font(Theme.mono(12)).foregroundStyle(Theme.warn)
                    Spacer()
                    Text(time(j.at)).font(Theme.mono(12)).foregroundStyle(Theme.muted)
                }
                .accessibilityElement(children: .combine)
            }
            if ops.jobs.isEmpty {
                Text("None.").font(.callout).foregroundStyle(Theme.muted)
            }
        }
    }

    /// Bars of `heights` (0...1), drawn with shapes and hidden from VoiceOver.
    private func bars(_ heights: [Double], accentLast: Bool) -> some View {
        HStack(alignment: .bottom, spacing: 3) {
            ForEach(Array(heights.enumerated()), id: \.offset) { i, h in
                RoundedRectangle(cornerRadius: 2)
                    .fill(accentLast && i == heights.count - 1 ? Theme.accent : Theme.ink)
                    .frame(height: max(2, 48 * h))
            }
        }
        .frame(maxWidth: .infinity, minHeight: 48, alignment: .bottomLeading)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.ink).frame(height: 2) }
        .accessibilityHidden(true)
    }

    // MARK: cost and errors

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
}
