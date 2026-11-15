// PigeonCore's learner scores: marks score as the key says, progress is weakest first, and the
// hints and words read as the screens show them.

import Foundation
import Testing

import PigeonCore

private let bank = [
    Question(
        id: 1, topic: "Parking", question: "Parking?", choices: ["3 m", "5 m", "6 m", "10 m"],
        answer: "B", explanation: "", source: ""),
    Question(
        id: 2, topic: "Signs", question: "Signs?", choices: ["Stop", "Yield", "Go", "Turn"],
        answer: "A", explanation: "", source: ""),
    Question(
        id: 3, topic: "Sharing the road", question: "Passing?",
        choices: ["Pedestrians", "Cyclists", "Motorcyclists", "All of the above"], answer: "D",
        explanation: "", source: ""),
]

private func attempt(_ marks: [[Int]]) -> Attempt {
    let key = [1, 0, 3]
    return Attempt(
        id: "a", form: 17, page: 0, date: 0, questionIDs: [1, 2, 3], marks: marks,
        results: score(key: key, marks: marks))
}

@Suite struct LearnerScoreTests {
    @Test func marksScoreAsTheKeySays() {
        #expect(
            score(key: [1, 0, 3, 2], marks: [[1], [2], [1, 3], []])
                == [.correct, .wrong, .double, .blank])
        #expect(score(key: [1, 0], marks: [[1]]) == [.correct, .blank])
    }

    @Test func passMarkIsEightyPercentRoundedUp() {
        #expect(passMark(25) == 20)
        #expect(passMark(50) == 40)
        #expect(passMark(3) == 3)
    }

    @Test func pageSliceTakesTwentyFivePerPage() {
        let form = Form(number: 1, questionIDs: Array(1...50), key: Array(repeating: 0, count: 50))
        #expect(pageSlice(form, page: 1).ids.first == 26)
        #expect(pageSlice(form, page: 1).ids.count == 25)
        #expect(pageSlice(form, page: 2).ids.isEmpty)
    }

    @Test func progressIsWeakestFirst() {
        let p = topicProgress(attempts: [attempt([[1], [0], [1, 3]])], bank: bank)
        #expect(p.map(\.topic) == ["Sharing the road", "Parking", "Signs"])
        #expect(p.first == TopicProgress(topic: "Sharing the road", right: 0, total: 1))
        #expect(missRates(attempts: [attempt([[1], [0], [1, 3]])], bank: bank)["Sharing the road"] == 1)
    }

    @Test func hintsCoverEveryMissedQuestion() {
        let h = hints(attempt: attempt([[1], [], [1, 3]]), bank: bank)
        #expect(h.map(\.number) == [2, 3])
        #expect(h.map(\.result) == [.blank, .double])
    }

    @Test func wordsReadAsTheScreensShow() {
        #expect(markedText([], question: bank[2]) == "Left blank")
        #expect(markedText([3, 1], question: bank[2]) == "B and D (both filled)")
        #expect(markedText([1], question: bank[2]) == "B · Cyclists")
        #expect(answerText(bank[2]) == "D · All of the above")
        #expect(
            encouragement(score: 23, count: 25, previousBest: 21)
                == "Nice work. That's your best score so far.")
        #expect(encouragement(score: 21, count: 25, previousBest: 23) == "You passed. Keep it up.")
        #expect(
            encouragement(score: 15, count: 25, previousBest: nil)
                == "Not there yet. The study tips show what to read next.")
    }
}
