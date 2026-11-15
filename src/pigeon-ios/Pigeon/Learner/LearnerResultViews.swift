// The learner's score and study-tip screens, and the buttons the learner's screens share; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §8 and step 2.

import PigeonCore
import SwiftUI

func learnerPrimary(_ title: String) -> some View {
    Text(title)
        .font(.headline)
        .foregroundStyle(Theme.onAccent)
        .frame(maxWidth: .infinity, minHeight: 52)
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.accent))
        .background(RoundedRectangle(cornerRadius: 14).fill(Theme.shadow).offset(x: 3, y: 3))
}

func learnerSecondary(_ title: String) -> some View {
    Text(title)
        .font(.subheadline.weight(.semibold))
        .foregroundStyle(Theme.ink)
        .frame(maxWidth: .infinity, minHeight: 44)
        .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
}

func formLabel(_ number: Int) -> String { String(format: "F%03d", number) }

/// The score, saved: pass or not yet, a kind line, and each question's result.
struct ResultView: View {
    @ObservedObject var learner: LearnerStore
    let attempt: Attempt
    let previousBest: Int?
    @State private var showHints = false

    var body: some View {
        let count = attempt.results.count
        let pass = attempt.correct >= passMark(count)
        let missed = count - attempt.correct
        let sample = isSample(attempt.form)
        return ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Eyebrow(text: sample ? "Sample page" : formLabel(attempt.form))
                    VStack(spacing: 8) {
                        HStack(alignment: .firstTextBaseline, spacing: 4) {
                            Text("\(attempt.correct)").font(Theme.mono(64, .bold)).foregroundStyle(Theme.ink)
                            Text("/\(count)").font(Theme.mono(22, .semibold)).foregroundStyle(Theme.muted)
                        }
                        Text(pass ? "PASS" : "NOT YET")
                            .font(Theme.mono(13, .bold))
                            .tracking(1)
                            .foregroundStyle(Theme.onAccent)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 4)
                            .background(Capsule().fill(pass ? Theme.good : Theme.kind(1)))
                        Text(
                            sample
                                ? "This was a sample page, so nothing was kept."
                                : encouragement(score: attempt.correct, count: count, previousBest: previousBest))
                            .font(.callout)
                            .foregroundStyle(Theme.ink2)
                            .multilineTextAlignment(.center)
                    }
                    .padding(20)
                    .frame(maxWidth: .infinity)
                    .block(shadow: pass ? Theme.good : Theme.shadow)
                    LazyVGrid(
                        columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 9), spacing: 6
                    ) {
                        ForEach(Array(attempt.results.enumerated()), id: \.offset) { i, r in
                            Text("\(attempt.page * questionsPerPage + i + 1)")
                                .font(Theme.mono(12, .semibold))
                                .frame(maxWidth: .infinity, minHeight: 30)
                                .foregroundStyle(r == .correct ? Theme.good : Theme.onAccent)
                                .background(
                                    RoundedRectangle(cornerRadius: 7).fill(r == .correct ? Theme.chip : Theme.kind(1)))
                        }
                    }
                    .padding(16)
                    .block()
                    Label(
                        sample ? "A sample: not saved, and not in your progress" : "Saved to your progress",
                        systemImage: sample ? "xmark" : "checkmark")
                        .font(.footnote)
                        .foregroundStyle(Theme.ink2)
                    if missed > 0 {
                        Button { showHints = true } label: {
                            learnerPrimary(missed == 1 ? "See 1 study tip" : "See \(missed) study tips")
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            }
        }
        .navigationDestination(isPresented: $showHints) {
            HintsView(hints: hints(attempt: attempt, bank: learner.questions(for: attempt)))
        }
    }
}

/// Study tips: for each missed question, what was marked, the answer, why, and where to read.
struct HintsView: View {
    let hints: [Hint]

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(hints.count == 1 ? "One thing to brush up on" : "\(hints.count) things to brush up on")
                        .font(Theme.display(22))
                        .foregroundStyle(Theme.ink)
                    Text("A few minutes with each section should do it.")
                        .font(.callout)
                        .foregroundStyle(Theme.muted)
                    ForEach(hints, id: \.number) { h in card(h) }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            }
        }
    }

    private func card(_ h: Hint) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text(h.question.topic)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Theme.accent)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Theme.accentSoft))
                Text("Q\(h.number)").font(Theme.mono(12)).foregroundStyle(Theme.muted)
            }
            Text(h.question.question).font(.callout.weight(.semibold)).foregroundStyle(Theme.ink)
            HStack(alignment: .top, spacing: 8) {
                answerBox("YOU MARKED", markedText(h.marked, question: h.question), Theme.kind(1))
                answerBox("ANSWER", answerText(h.question), Theme.good)
            }
            Text(h.question.explanation).font(.callout).foregroundStyle(Theme.ink2)
            Divider()
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "book").foregroundStyle(Theme.accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Where to read").font(.footnote.weight(.semibold)).foregroundStyle(Theme.ink)
                    Text(h.question.source).font(.footnote).foregroundStyle(Theme.ink2)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .block()
    }

    private func answerBox(_ label: String, _ value: String, _ color: Color) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(Theme.mono(11, .semibold)).foregroundStyle(color)
            Text(value).font(.footnote).foregroundStyle(Theme.ink)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
    }
}
