// PigeonCore's learner model: forms draw the right mix the same way every time, and the code strip
// round-trips.

import Foundation
import Testing

import PigeonCore

private let topics = [
    "Alcohol & drugs", "Emergencies", "GLP (L/N rules)", "Hazards & conditions",
    "Licensing & penalties", "Parking", "Right-of-way", "Sharing the road", "Signals & lanes",
    "Signs", "Speed",
]

/// 12 questions per topic: the 11th of Parking, Speed and Emergencies is combined, every 12th unverified.
private func syntheticBank() -> [Question] {
    var out: [Question] = []
    var id = 1
    for t in topics {
        for i in 0..<12 {
            let combined = ["Parking", "Speed", "Emergencies"].contains(t) && i == 10
            out.append(
                Question(
                    id: id, topic: t, question: "Question \(id) about \(t)?",
                    choices: ["One \(i)", "Two", "Three", "Four"], answer: choiceLetters[id % 4],
                    explanation: "", source: "", unverified: i == 11 ? true : nil,
                    combined: combined ? true : nil))
            id += 1
        }
    }
    return out
}

@Suite struct LearnerTests {
    @Test func formDrawsTwentyFiveVerifiedQuestions() {
        let bank = syntheticBank()
        let byID = Dictionary(uniqueKeysWithValues: bank.map { ($0.id, $0) })
        let qs = makeForm(bank: bank, number: 17).questionIDs.compactMap { byID[$0] }
        #expect(qs.count == 25)
        #expect(Set(qs.map(\.id)).count == 25)
        #expect(qs.allSatisfy { $0.unverified != true })
        #expect(qs.filter { $0.topic == "Signs" }.count == 5)
        #expect(qs.filter { $0.combined == true }.count == 3)
    }

    @Test func formIsDeterministic() {
        let bank = syntheticBank()
        #expect(makeForm(bank: bank, number: 17) == makeForm(bank: bank, number: 17))
        #expect(makeForm(bank: bank, number: 17) != makeForm(bank: bank, number: 18))
    }

    @Test func keyIsEachQuestionsAnswer() {
        let bank = syntheticBank()
        let byID = Dictionary(uniqueKeysWithValues: bank.map { ($0.id, $0) })
        let form = makeForm(bank: bank, number: 3)
        for (id, k) in zip(form.questionIDs, form.key) {
            #expect(byID[id]?.answerIndex == k)
        }
    }

    @Test func missedTopicsAreDrawnMoreOften() {
        let bank = syntheticBank()
        let byID = Dictionary(uniqueKeysWithValues: bank.map { ($0.id, $0) })
        func parking(_ f: Form) -> Int {
            f.questionIDs.compactMap { byID[$0] }
                .filter { $0.topic == "Parking" && $0.combined != true }.count
        }
        let plain = makeForm(bank: bank, number: 5)
        let weighted = makeForm(bank: bank, number: 5, missRates: ["Parking": 0.5])
        #expect(parking(weighted) > parking(plain))
    }

    @Test func codeBitsRoundTrip() {
        #expect(
            codeBits(form: 17, page: 0)
                == [false, false, false, false, false, true, false, false, false, true, false, false])
        for form in 0..<1024 {
            for page in 0..<4 {
                let decoded = decodeCode(codeBits(form: form, page: page))
                #expect(decoded?.form == form && decoded?.page == page)
            }
        }
        #expect(decodeCode([true]) == nil)
    }
}
