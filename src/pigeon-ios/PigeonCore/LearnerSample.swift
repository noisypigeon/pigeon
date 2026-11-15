// The learner's sample page, form 1023, and the ledger that keeps real pages and never the sample; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md, its 2026-11-15 sample amendment.

import Foundation

/// The sample's form number: all ten form bits of the code strip set. Real forms never take it.
public let sampleFormNumber = 1023

/// Whether `form` is the sample.
public func isSample(_ form: Int) -> Bool { form == sampleFormNumber }

private let sampleRows: [(String, [String], String)] = [
    ("Which colour is the sky?", ["Green", "Blue", "Purple", "Orange"], "B"),
    ("How many legs does a cat have?", ["Two", "Six", "Four", "Eight"], "C"),
    ("Which of these is a fruit?", ["Apple", "Brick", "Spoon", "Cloud"], "A"),
    ("What is 2 + 2?", ["Three", "Five", "Six", "Four"], "D"),
    ("Which animal says moo?", ["Cow", "Duck", "Fish", "Owl"], "A"),
    ("What colour is snow?", ["Red", "White", "Black", "Blue"], "B"),
    ("How many days are in a week?", ["Five", "Six", "Seven", "Ten"], "C"),
    ("Which one can fly?", ["Rock", "Chair", "Sock", "Bird"], "D"),
    ("What do bees make?", ["Honey", "Milk", "Bread", "Glue"], "A"),
    ("Which is the largest?", ["Ant", "Elephant", "Mouse", "Frog"], "B"),
    ("What shape is a wheel?", ["Square", "Triangle", "Round", "Star"], "C"),
    ("Which is a vegetable?", ["Pebble", "Lamp", "Shoe", "Carrot"], "D"),
    ("What day comes after Monday?", ["Tuesday", "Friday", "Sunday", "Saturday"], "A"),
    ("What is frozen water?", ["Sand", "Ice", "Smoke", "Wool"], "B"),
    ("Which of these is hot?", ["Snow", "Ice cream", "The sun", "Rain"], "C"),
    ("How many wheels does a bicycle have?", ["One", "Three", "Four", "Two"], "D"),
    ("Which is a musical instrument?", ["Guitar", "Pillow", "Carpet", "Fork"], "A"),
    ("What do you see with?", ["Ears", "Eyes", "Elbows", "Knees"], "B"),
    ("Which month comes first?", ["June", "March", "January", "May"], "C"),
    ("Where do fish live?", ["Trees", "Clouds", "Deserts", "Water"], "D"),
    ("What is 10 - 5?", ["Five", "Two", "Eight", "Ten"], "A"),
    ("Which colour is grass?", ["Pink", "Green", "Grey", "Gold"], "B"),
    ("What do you sleep in?", ["A sink", "A shoe", "A bed", "A cup"], "C"),
    ("Which is a pet?", ["Cactus", "Kettle", "Bus", "Dog"], "D"),
    ("How many hours are in a day?", ["24", "12", "7", "100"], "A"),
]

/// The sample's 25 questions, each plainly not the knowledge test's.
public let sampleQuestions: [Question] = sampleRows.enumerated().map { i, row in
    Question(
        id: 90_001 + i, topic: "Sample", question: "Sample \(i + 1): \(row.0)", choices: row.1,
        answer: row.2, explanation: "A sample question, so there is nothing to study here.",
        source: "Sample page, not the knowledge test")
}

/// The sample form: its questions in order, one page.
public let sampleForm = Form(
    number: sampleFormNumber, questionIDs: sampleQuestions.map(\.id),
    key: sampleQuestions.map(\.answerIndex))

/// The sample page laid out without wrapping, so its circles fall in the same place on every device.
public let sampleLayout: PageLayout = layoutPage(sampleQuestions, first: 0) { text, _, _, _ in [text] }

/// The filled-in sample's marks: 21 right, 4 and 11 wrong, 17 double and 23 blank.
public let sampleMarks: [[Int]] = sampleQuestions.enumerated().map { i, q in
    switch i + 1 {
    case 4, 11: return [0]
    case 17: return [0, 1]
    case 23: return []
    default: return [q.answerIndex]
    }
}

/// The circles the filled-in sample has filled.
public let sampleFills: [PlacedCircle] = sampleLayout.circles.filter {
    sampleMarks[$0.question].contains($0.choice)
}

/// The learner's practice on the phone: printed forms, each page's circles, and scored pages. The
/// sample is read and scored through it like any form, but never kept.
public struct LearnerLedger: Codable, Equatable, Sendable {
    public private(set) var forms: [Form]
    public private(set) var attempts: [Attempt]
    public private(set) var circles: [String: [PlacedCircle]]

    public init(forms: [Form] = [], attempts: [Attempt] = [], circles: [String: [PlacedCircle]] = [:]) {
        self.forms = forms
        self.attempts = attempts
        self.circles = circles
    }

    /// The key a printed page's circles are kept under.
    public static func pageKey(form: Int, page: Int) -> String { "\(form)-\(page)" }

    /// The best score so far, or nil before the first.
    public var best: Int? { attempts.map(\.correct).max() }

    /// The next form's number, after the newest; it wraps before the sample's.
    public var nextFormNumber: Int { ((forms.last?.number ?? 16) + 1) % sampleFormNumber }

    /// Form `number`: the sample, or the newest printed form with that number.
    public func form(_ number: Int) -> Form? {
        isSample(number) ? sampleForm : forms.last { $0.number == number }
    }

    /// The circles page `page` of form `form` was printed with, as the reader's `circles` wants them.
    public func pageCircles(form: Int, page: Int) -> [PlacedCircle]? {
        if isSample(form) { return page == 0 ? sampleLayout.circles : nil }
        return circles[Self.pageKey(form: form, page: page)]
    }

    /// Keeps a printed form and each of its pages' circles; the sample is never kept.
    public mutating func add(_ form: Form, circles pages: [[PlacedCircle]]) {
        guard !isSample(form.number) else { return }
        forms.append(form)
        for (page, list) in pages.enumerated() {
            circles[Self.pageKey(form: form.number, page: page)] = list
        }
    }

    /// Scores `marks` for a printed page. A real page's attempt is kept; the sample's is only returned.
    public mutating func record(
        form number: Int, page: Int, marks: [[Int]], id: String, date: Int
    ) -> Attempt? {
        guard let f = form(number) else { return nil }
        let slice = pageSlice(f, page: page)
        guard !slice.ids.isEmpty else { return nil }
        let attempt = Attempt(
            id: id, form: number, page: page, date: date, questionIDs: slice.ids, marks: marks,
            results: score(key: slice.key, marks: marks))
        if !isSample(number) { attempts.append(attempt) }
        return attempt
    }
}
