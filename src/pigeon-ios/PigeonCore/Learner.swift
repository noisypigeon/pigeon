// The learner's practice tests: the bank's model, the form generator and the code strip;
// see .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §3 and its 2026-11-15 amendment.

import Foundation

/// The choices' letters, in order.
public let choiceLetters = ["A", "B", "C", "D"]

/// One question of the bank, as `Pigeon/Learner/questions.json` holds it.
public struct Question: Codable, Equatable, Sendable, Identifiable {
    public let id: Int
    public let topic: String
    public let question: String
    public let choices: [String]
    public let answer: String
    public let explanation: String
    public let source: String
    public let unverified: Bool?
    public let combined: Bool?

    public init(
        id: Int, topic: String, question: String, choices: [String], answer: String,
        explanation: String, source: String, unverified: Bool? = nil, combined: Bool? = nil
    ) {
        self.id = id
        self.topic = topic
        self.question = question
        self.choices = choices
        self.answer = answer
        self.explanation = explanation
        self.source = source
        self.unverified = unverified
        self.combined = combined
    }

    /// The correct choice, 0 for A to 3 for D.
    public var answerIndex: Int { choiceLetters.firstIndex(of: answer) ?? 0 }
}

/// The bank, decoded from its JSON.
public func decodeBank(_ data: Data) throws -> [Question] {
    try JSONDecoder().decode([Question].self, from: data)
}

/// Mulberry32: a small seeded generator, so a form number always draws the same questions.
public struct Mulberry32: Sendable {
    private var state: UInt32

    public init(seed: UInt32) { state = seed }

    /// The next value in [0, 1).
    public mutating func next() -> Double {
        state = state &+ 0x6D2B_79F5
        let s = state
        var t = (s ^ (s >> 15)) &* (s | 1)
        t = (t &+ ((t ^ (t >> 7)) &* (t | 61))) ^ t
        return Double(t ^ (t >> 14)) / 4_294_967_296
    }
}

/// `items` in an order drawn from `rng` (Fisher–Yates).
public func learnerShuffled<T>(_ items: [T], _ rng: inout Mulberry32) -> [T] {
    var a = items
    var i = a.count - 1
    while i > 0 {
        let j = Int(rng.next() * Double(i + 1))
        a.swapAt(i, j)
        i -= 1
    }
    return a
}

/// A printed test: its number, its questions in order, and each one's correct choice.
public struct Form: Codable, Equatable, Sendable {
    public let number: Int
    public let questionIDs: [Int]
    public let key: [Int]

    public init(number: Int, questionIDs: [Int], key: [Int]) {
        self.number = number
        self.questionIDs = questionIDs
        self.key = key
    }
}

/// Form `number`: a fifth sign questions, a tenth combined-answer questions, the rest round-robin
/// across topics, a topic missed a quarter of the time or more drawing two per round. Unverified
/// questions are never drawn. The same inputs always give the same form.
public func makeForm(
    bank: [Question], number: Int, count: Int = 25, missRates: [String: Double] = [:]
) -> Form {
    var rng = Mulberry32(seed: UInt32(truncatingIfNeeded: number &* 7919 &+ count))
    let pool = bank.filter { $0.unverified != true }
    let signs = learnerShuffled(pool.filter { $0.topic == "Signs" && $0.combined != true }, &rng)
        .prefix(Int((Double(count) / 5).rounded()))
    let combined = learnerShuffled(pool.filter { $0.combined == true }, &rng)
        .prefix(Int((Double(count) / 10).rounded(.up)))
    var byTopic: [String: [Question]] = [:]
    for q in pool where q.topic != "Signs" && q.combined != true {
        byTopic[q.topic, default: []].append(q)
    }
    let topics = byTopic.keys.sorted {
        let a = missRates[$0] ?? 0, b = missRates[$1] ?? 0
        return a != b ? a > b : $0 < $1
    }
    for t in topics { byTopic[t] = learnerShuffled(byTopic[t] ?? [], &rng) }
    var rest = Array(combined)
    let need = count - signs.count
    var taken: [String: Int] = [:]
    var progress = true
    while rest.count < need && progress {
        progress = false
        for t in topics where rest.count < need {
            let picks = (missRates[t] ?? 0) >= 0.25 ? 2 : 1
            for _ in 0..<picks where rest.count < need {
                let i = taken[t, default: 0]
                if let list = byTopic[t], i < list.count {
                    rest.append(list[i])
                    taken[t] = i + 1
                    progress = true
                }
            }
        }
    }
    let qs = learnerShuffled(Array(signs) + rest, &rng)
    return Form(number: number, questionIDs: qs.map(\.id), key: qs.map(\.answerIndex))
}

/// The code strip's 12 bits, most significant first: 10 for the form, 2 for the page (0 to 3).
public func codeBits(form: Int, page: Int) -> [Bool] {
    (0..<10).map { (form >> (9 - $0)) & 1 == 1 } + (0..<2).map { (page >> (1 - $0)) & 1 == 1 }
}

/// The form and page a code strip's 12 bits name, or nil for any other count.
public func decodeCode(_ bits: [Bool]) -> (form: Int, page: Int)? {
    guard bits.count == 12 else { return nil }
    let v = bits.reduce(0) { ($0 << 1) | ($1 ? 1 : 0) }
    return (v >> 2, v & 3)
}
