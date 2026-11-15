// The learner's scores: a page's marks against its form's key, progress by topic, and the hints for
// missed questions; see .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §4 and §5.

import Foundation

/// How one question was answered.
public enum MarkResult: String, Codable, Sendable {
    case correct, wrong, double, blank
}

/// One scored page: its form and page, when, which circles were filled per question, and each result.
public struct Attempt: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let form: Int
    public let page: Int
    public let date: Int
    public let questionIDs: [Int]
    public let marks: [[Int]]
    public let results: [MarkResult]

    public init(
        id: String, form: Int, page: Int, date: Int, questionIDs: [Int], marks: [[Int]],
        results: [MarkResult]
    ) {
        self.id = id
        self.form = form
        self.page = page
        self.date = date
        self.questionIDs = questionIDs
        self.marks = marks
        self.results = results
    }

    public var correct: Int { results.filter { $0 == .correct }.count }
}

/// The questions per printed page.
public let questionsPerPage = 25

/// The form's question IDs and key for `page` (0-based).
public func pageSlice(_ form: Form, page: Int) -> (ids: [Int], key: [Int]) {
    let start = min(page * questionsPerPage, form.questionIDs.count)
    let end = min(start + questionsPerPage, form.questionIDs.count)
    return (Array(form.questionIDs[start..<end]), Array(form.key[start..<end]))
}

/// Each question's result: one mark on the key is correct, one elsewhere wrong, two or more double
/// (scored as wrong), none blank.
public func score(key: [Int], marks: [[Int]]) -> [MarkResult] {
    key.enumerated().map { i, k in
        let m = i < marks.count ? marks[i] : []
        if m.count > 1 { return .double }
        guard let only = m.first else { return .blank }
        return only == k ? .correct : .wrong
    }
}

/// The pass mark for `count` questions: 80%, rounded up.
public func passMark(_ count: Int) -> Int { Int((Double(count) * 0.8).rounded(.up)) }

/// Right answers out of questions seen, for one topic.
public struct TopicProgress: Equatable, Sendable {
    public let topic: String
    public let right: Int
    public let total: Int

    public init(topic: String, right: Int, total: Int) {
        self.topic = topic
        self.right = right
        self.total = total
    }
}

/// Every topic answered in `attempts`, weakest first (by share right, then by name).
public func topicProgress(attempts: [Attempt], bank: [Question]) -> [TopicProgress] {
    let byID = Dictionary(bank.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
    var right: [String: Int] = [:]
    var total: [String: Int] = [:]
    for a in attempts {
        for (id, r) in zip(a.questionIDs, a.results) {
            guard let q = byID[id] else { continue }
            total[q.topic, default: 0] += 1
            if r == .correct { right[q.topic, default: 0] += 1 }
        }
    }
    return total.keys.map { TopicProgress(topic: $0, right: right[$0] ?? 0, total: total[$0] ?? 0) }
        .sorted {
            let a = Double($0.right) / Double($0.total), b = Double($1.right) / Double($1.total)
            return a != b ? a < b : $0.topic < $1.topic
        }
}

/// Each topic's share of answers that were not correct, for `makeForm`'s weighting.
public func missRates(attempts: [Attempt], bank: [Question]) -> [String: Double] {
    var out: [String: Double] = [:]
    for t in topicProgress(attempts: attempts, bank: bank) {
        out[t.topic] = Double(t.total - t.right) / Double(t.total)
    }
    return out
}

/// A missed question on a scored page: its number on the form (1-based), the question, what was
/// marked, and the result.
public struct Hint: Equatable, Sendable {
    public let number: Int
    public let question: Question
    public let marked: [Int]
    public let result: MarkResult

    public init(number: Int, question: Question, marked: [Int], result: MarkResult) {
        self.number = number
        self.question = question
        self.marked = marked
        self.result = result
    }
}

/// The hints for every question on `attempt` that was not correct, in page order.
public func hints(attempt: Attempt, bank: [Question]) -> [Hint] {
    let byID = Dictionary(bank.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
    var out: [Hint] = []
    for (i, id) in attempt.questionIDs.enumerated() where attempt.results[i] != .correct {
        guard let q = byID[id] else { continue }
        out.append(
            Hint(
                number: attempt.page * questionsPerPage + i + 1, question: q,
                marked: attempt.marks[i], result: attempt.results[i]))
    }
    return out
}

/// What was marked, in words: "Left blank", "B and D (both filled)", or "B · Cyclists".
public func markedText(_ marks: [Int], question: Question) -> String {
    if marks.isEmpty { return "Left blank" }
    if marks.count > 1 {
        let letters = marks.sorted().map { choiceLetters[$0] }
        return letters.joined(separator: " and ") + " (both filled)"
    }
    return choiceLetters[marks[0]] + " · " + question.choices[marks[0]]
}

/// The right answer, in words: "D · All of the above".
public func answerText(_ question: Question) -> String {
    choiceLetters[question.answerIndex] + " · " + question.choices[question.answerIndex]
}

/// The line under a score: kind, and never a judgement.
public func encouragement(score: Int, count: Int, previousBest: Int?) -> String {
    let pass = score >= passMark(count)
    if pass, previousBest.map({ score > $0 }) ?? true {
        return "Nice work. That's your best score so far."
    }
    if pass { return "You passed. Keep it up." }
    return "Not there yet. The study tips show what to read next."
}
