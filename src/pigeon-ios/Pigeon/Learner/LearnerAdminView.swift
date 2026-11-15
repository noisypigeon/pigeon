// A learner's instance as its administrator sees it: their progress, read-only, and inviting and
// admitting the learner, each write proved by passkey on that instance; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §8 and its 2026-11-15
// amendment, §7.

import PigeonCore
import SwiftUI

/// `GET /v0/app/learner/progress`'s answer.
struct LearnerProgress: Decodable {
    struct Entry: Decodable, Identifiable {
        let id: String
        let form: Int
        let page: Int
        let date: Int
        let correct: Int
        let count: Int
    }

    let attempts: [Entry]
    let progress: [String: QuestionTally]
}

@MainActor
final class LearnerAdmin: ObservableObject {
    @Published var progress: LearnerProgress?
    @Published var attempts: [Attempt] = []
    @Published var invitations: [InvitationRow] = []
    @Published var link: URL?
    @Published var error: String?
    @Published var busy = false
    let instance: InstanceEntry
    let bank: [Question]
    private let auth = Auth()

    init(instance: InstanceEntry) {
        self.instance = instance
        let url = Bundle.main.url(forResource: "questions", withExtension: "json")
        bank = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? decodeBank($0) } ?? []
    }

    private func api() throws -> API {
        guard let b = instance.base_url, let base = URL(string: b) else {
            throw APIError(status: 0, message: "This instance isn't ready yet.")
        }
        return API(device: try DeviceKey.load(), base: base)
    }

    func load() async {
        do {
            let api = try api()
            progress = try await api.call("GET", "/v0/app/learner/progress", as: LearnerProgress.self)
            attempts = try await api.call("GET", "/v0/app/learner/attempts", as: AttemptList.self).attempts
            invitations = try await api.call("GET", "/v0/app/members", as: MembersView.self).invitations
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Topics by share right, weakest first, from the instance's per-question record.
    var topics: [TopicProgress] {
        let byID = Dictionary(bank.map { ($0.id, $0.topic) }, uniquingKeysWith: { a, _ in a })
        var right: [String: Int] = [:]
        var total: [String: Int] = [:]
        for (id, t) in progress?.progress ?? [:] {
            guard let n = Int(id), let topic = byID[n] else { continue }
            right[topic, default: 0] += t.right
            total[topic, default: 0] += t.seen
        }
        return total.keys.map { TopicProgress(topic: $0, right: right[$0] ?? 0, total: total[$0] ?? 0) }
            .sorted { Double($0.right) / Double($0.total) < Double($1.right) / Double($1.total) }
    }

    private func proved(op: String, id: String) async throws -> PasskeyAnswer {
        let c = try await api().call(
            "POST", "/v0/app/challenge", body: ["op": op, "id": id], as: PasskeyChallenge.self)
        return try await auth.prove(c)
    }

    private struct Invited: Decodable {
        let link: String
    }

    /// Invites `email` as a User, with a passkey; the link to send them.
    func invite(email: String) async {
        busy = true
        defer { busy = false }
        do {
            let answer = try await proved(op: "member:invite", id: email.trimmingCharacters(in: .whitespaces).lowercased())
            let invited = try await api().call(
                "POST", "/v0/app/members/invites", body: ["email": email, "passkey": answer.json],
                as: Invited.self)
            link = URL(string: invited.link)
            await load()
        } catch AuthError.cancelled {
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Admits invitation `id` once its code matches, with a passkey.
    func admit(_ id: String) async {
        busy = true
        defer { busy = false }
        do {
            let answer = try await proved(op: "member:admit", id: id)
            try await api().send("POST", "/v0/app/members/admit", body: ["id": id, "passkey": answer.json])
            await load()
        } catch AuthError.cancelled {
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// The learner's instance for its administrator: Progress and Hints, read-only, and its member.
struct LearnerAdminView: View {
    @StateObject private var admin: LearnerAdmin
    @State private var email = ""
    @Environment(\.dismiss) private var dismiss

    init(instance: InstanceEntry) { _admin = StateObject(wrappedValue: LearnerAdmin(instance: instance)) }

    var body: some View {
        NavigationStack {
            ZStack {
                DotGrid()
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Eyebrow(text: "Class 7L practice")
                        Text(admin.instance.name).font(Theme.display(22)).foregroundStyle(Theme.ink)
                        if let e = admin.error {
                            Text(e).font(.footnote).foregroundStyle(Theme.kind(1))
                        }
                        scores
                        weakest
                        missed
                        members
                    }
                    .padding(20)
                }
                .refreshable { await admin.load() }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
            }
            .task { await admin.load() }
        }
    }

    private var scores: some View {
        let recent = Array((admin.progress?.attempts ?? []).suffix(8))
        return VStack(alignment: .leading, spacing: 8) {
            Text(recent.isEmpty ? "No tests scored yet" : "Last \(recent.count) tests")
                .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
            HStack(alignment: .bottom, spacing: 8) {
                ForEach(recent) { a in
                    VStack(spacing: 4) {
                        Text("\(a.correct)/\(a.count)").font(Theme.mono(11, .semibold)).foregroundStyle(Theme.ink)
                        RoundedRectangle(cornerRadius: 4)
                            .fill(a.correct >= passMark(max(a.count, 1)) ? Theme.good : Theme.kind(1))
                            .frame(height: max(4, 100 * Double(a.correct) / Double(max(a.count, 1))))
                        Text(Date(timeIntervalSince1970: TimeInterval(a.date)), format: .dateTime.month().day())
                            .font(Theme.mono(9)).foregroundStyle(Theme.muted)
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .frame(height: 140, alignment: .bottom)
        }
    }

    private var weakest: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Topics to brush up on").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
            ForEach(admin.topics.prefix(4), id: \.topic) { t in
                HStack {
                    Text(t.topic).font(.footnote).foregroundStyle(Theme.ink)
                    Spacer()
                    Text("\(t.right) of \(t.total)").font(Theme.mono(12)).foregroundStyle(Theme.muted)
                }
            }
        }
    }

    private var missed: some View {
        let missed = admin.attempts.last.map { PigeonCore.hints(attempt: $0, bank: admin.bank) } ?? []
        return VStack(alignment: .leading, spacing: 6) {
            Text("Last test's missed questions").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
            if missed.isEmpty {
                Text("None yet.").font(.footnote).foregroundStyle(Theme.muted)
            }
            ForEach(Array(missed.prefix(5).enumerated()), id: \.offset) { _, h in
                VStack(alignment: .leading, spacing: 2) {
                    Text(h.question.question).font(.footnote.weight(.semibold)).foregroundStyle(Theme.ink)
                    Text(h.question.explanation).font(.footnote).foregroundStyle(Theme.muted)
                }
            }
        }
    }

    private var members: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("The learner").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
            ForEach(admin.invitations) { i in
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(i.email) · \(i.status)").font(.footnote).foregroundStyle(Theme.ink)
                    if i.status == "enrolled", let code = i.match_code {
                        Text("Their screen should show \(code.prefix(3)) \(code.suffix(3))")
                            .font(Theme.mono(14, .semibold)).foregroundStyle(Theme.ink)
                        Button { Task { await admin.admit(i.id) } } label: {
                            learnerPrimary("It matches: admit")
                        }
                        .disabled(admin.busy)
                    }
                }
            }
            if let link = admin.link {
                ShareLink(item: link) { learnerSecondary("Send the invitation link") }
                Text("It opens Pigeon on their iPhone, and works once, for 3 days.")
                    .font(.footnote).foregroundStyle(Theme.muted)
            }
            TextField("Their email", text: $email)
                .textFieldStyle(.roundedBorder)
                .keyboardType(.emailAddress)
                .textInputAutocapitalization(.never)
            Button { Task { await admin.invite(email: email) } } label: { learnerSecondary("Invite") }
                .disabled(admin.busy || !email.contains("@"))
        }
    }
}
