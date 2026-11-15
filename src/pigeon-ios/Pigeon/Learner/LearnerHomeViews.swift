// The learner's progress and answer-entry screens; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §8 and step 2.

import PigeonCore
import SwiftUI
import UIKit

/// Progress: the last six scores against the pass line, the weakest topics, and the two actions.
struct LearnerHomeView: View {
    @StateObject private var learner = LearnerStore()
    @Environment(\.dismiss) private var dismiss
    @State private var entering = false
    @State private var scanning = false

    var body: some View {
        NavigationStack {
            ZStack {
                DotGrid()
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        VStack(alignment: .leading, spacing: 2) {
                            Eyebrow(text: "Class 7L practice")
                            Text("Progress").font(Theme.display(22)).foregroundStyle(Theme.ink)
                        }
                        chart
                        topics
                        Button { printTest() } label: { learnerPrimary("Print a new test") }
                        Button { scanning = true } label: { learnerSecondary("Scan a page") }
                            .disabled(learner.forms.isEmpty)
                        Button { entering = true } label: { learnerSecondary("Enter answers") }
                            .disabled(learner.forms.isEmpty)
                    }
                    .padding(.horizontal, 20)
                    .padding(.vertical, 16)
                }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
            }
            .navigationDestination(isPresented: $entering) { AnswerEntryView(learner: learner) }
            .navigationDestination(isPresented: $scanning) { LearnerScanView(learner: learner) }
        }
    }

    private var chart: some View {
        let recent = Array(learner.attempts.suffix(6))
        return VStack(alignment: .leading, spacing: 10) {
            Text(recent.isEmpty ? "No tests scored yet" : "Last \(recent.count) tests")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Theme.ink)
            if recent.isEmpty {
                Text("Print a test, take it with a pencil, then scan it or enter your answers.")
                    .font(.footnote)
                    .foregroundStyle(Theme.muted)
            } else {
                HStack(alignment: .bottom, spacing: 10) {
                    ForEach(recent) { a in
                        let count = max(a.results.count, 1)
                        VStack(spacing: 4) {
                            Text("\(a.correct)").font(Theme.mono(13, .semibold)).foregroundStyle(Theme.ink)
                            RoundedRectangle(cornerRadius: 4)
                                .fill(a.correct >= passMark(count) ? Theme.good : Theme.kind(1))
                                .frame(height: max(4, 110 * Double(a.correct) / Double(count)))
                            Text(formLabel(a.form)).font(Theme.mono(10)).foregroundStyle(Theme.muted)
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
                .frame(height: 150, alignment: .bottom)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .block()
    }

    private var topics: some View {
        let weakest = Array(topicProgress(attempts: learner.attempts, bank: learner.bank).prefix(3))
        return VStack(alignment: .leading, spacing: 10) {
            Text("Topics to brush up on")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Theme.ink)
            if weakest.isEmpty {
                Text("These appear after your first scored test.")
                    .font(.footnote)
                    .foregroundStyle(Theme.muted)
            }
            ForEach(weakest, id: \.topic) { t in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(t.topic).font(.callout).foregroundStyle(Theme.ink)
                        Spacer()
                        Text("\(t.right)/\(t.total)").font(Theme.mono(12)).foregroundStyle(Theme.muted)
                    }
                    GeometryReader { g in
                        ZStack(alignment: .leading) {
                            Capsule().fill(Theme.chip)
                            Capsule()
                                .fill(Theme.kind(1))
                                .frame(width: g.size.width * Double(t.right) / Double(max(t.total, 1)))
                        }
                    }
                    .frame(height: 8)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .block()
    }

    private func printTest() {
        let data = learner.printNew(count: questionsPerPage)
        let info = UIPrintInfo.printInfo()
        info.outputType = .grayscale
        info.jobName = "Class 7L practice test"
        let controller = UIPrintInteractionController.shared
        controller.printInfo = info
        controller.printingItem = data
        controller.present(animated: true)
    }
}

/// Typing in a printed page's answers, one tap per question; tapping a chosen letter again clears it.
struct AnswerEntryView: View {
    @ObservedObject var learner: LearnerStore
    @State private var formNumber: Int?
    @State private var page = 0
    @State private var marks: [Int?] = Array(repeating: nil, count: questionsPerPage)
    @State private var result: Attempt?
    @State private var previousBest: Int?
    @State private var showResult = false

    private var form: PigeonCore.Form? { formNumber.flatMap { learner.form($0) } ?? learner.forms.last }
    private var pages: Int {
        form.map { max(1, ($0.questionIDs.count + questionsPerPage - 1) / questionsPerPage) } ?? 1
    }
    private var count: Int { form.map { pageSlice($0, page: page).ids.count } ?? 0 }

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Enter answers").font(Theme.display(22)).foregroundStyle(Theme.ink)
                    Picker(
                        "Test",
                        selection: Binding(
                            get: { form?.number ?? 0 },
                            set: {
                                formNumber = $0
                                page = 0
                                reset()
                            })
                    ) {
                        ForEach(learner.forms.reversed(), id: \.number) { f in
                            Text(formLabel(f.number)).tag(f.number)
                        }
                    }
                    .pickerStyle(.menu)
                    if pages > 1 {
                        Picker(
                            "Page",
                            selection: Binding(
                                get: { page },
                                set: {
                                    page = $0
                                    reset()
                                })
                        ) {
                            ForEach(0..<pages, id: \.self) { p in Text("Page \(p + 1)").tag(p) }
                        }
                        .pickerStyle(.segmented)
                    }
                    ForEach(0..<count, id: \.self) { i in row(i) }
                    Button { scoreIt() } label: { learnerPrimary("Score it") }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            }
        }
        .navigationDestination(isPresented: $showResult) {
            if let result {
                ResultView(learner: learner, attempt: result, previousBest: previousBest)
            }
        }
    }

    private func row(_ i: Int) -> some View {
        let number = page * questionsPerPage + i + 1
        return HStack(spacing: 8) {
            Text("\(number)")
                .font(Theme.mono(13, .semibold))
                .foregroundStyle(Theme.ink)
                .frame(width: 30, alignment: .trailing)
            ForEach(0..<4, id: \.self) { k in
                Button {
                    marks[i] = marks[i] == k ? nil : k
                } label: {
                    Text(choiceLetters[k])
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .foregroundStyle(marks[i] == k ? Theme.onAccent : Theme.ink)
                        .background(
                            RoundedRectangle(cornerRadius: 10).fill(marks[i] == k ? Theme.accent : Theme.chip))
                }
                .accessibilityLabel("Question \(number), \(choiceLetters[k])")
            }
        }
    }

    private func reset() { marks = Array(repeating: nil, count: questionsPerPage) }

    private func scoreIt() {
        guard let f = form else { return }
        previousBest = learner.best
        let entered: [[Int]] = (0..<count).map { i in marks[i].map { [$0] } ?? [] }
        result = learner.record(form: f.number, page: page, marks: entered)
        showResult = result != nil
    }
}
