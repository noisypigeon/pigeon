// Joining a learner's instance from its invitation link: a passkey made on this iPhone, the device's
// key signed over the link, the match code shown, then the wait for the administrator to admit;
// see .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md, its 2026-11-15
// amendment, §7.

import PigeonCore
import SwiftUI
import UIKit

@MainActor
final class LearnerJoin: ObservableObject {
    enum Stage: Equatable {
        case ready, working, waiting(code: String), joined, failed(String)
    }

    @Published var stage: Stage = .ready
    let link: JoinLink
    private let auth = Auth()

    init(link: JoinLink) { self.link = link }

    private func post<T: Decodable>(_ path: String, _ body: [String: Any], as: T.Type) async throws -> T {
        var request = URLRequest(url: URL(string: path, relativeTo: link.base)!)
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            throw APIError(status: status, message: String(decoding: data, as: UTF8.self))
        }
        return try JSONDecoder().decode(T.self, from: data)
    }

    private struct Challenge: Decodable {
        let challenge: String
        let rp: String
    }

    private struct JoinState: Decodable {
        let status: String
    }

    /// Makes the passkey, offers this iPhone's key, and shows the match code.
    func join(name: String) async {
        stage = .working
        do {
            let c = try await post(
                "/v0/app/join/challenge", ["token": link.token], as: Challenge.self)
            guard let challenge = fromBase64url(c.challenge) else { throw AuthError.badChallenge }
            let made = try await auth.create(
                challenge: challenge, rp: c.rp, name: name.isEmpty ? "Practice" : name,
                user: Data(UUID().uuidString.utf8))
            guard let key = attestedKey(attestationObject: made.attestation) else {
                throw APIError(status: 0, message: "The passkey's key couldn't be read.")
            }
            let device = try DeviceKey.load()
            let signature = try device.sign(joinMessage(token: link.token))
            let request = JoinRequest(
                token: link.token, name: name, deviceName: UIDevice.current.name,
                passkeyID: base64url(made.id), client: base64url(made.client),
                authenticator: base64url(key.authenticator), spki: base64url(spki(point: key.point)),
                deviceKey: device.publicKey, deviceSignature: base64url(signature))
            let body = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any] ?? [:]
            _ = try await post("/v0/app/join", body, as: Joined.self)
            stage = .waiting(code: matchCode(token: link.token, publicKey: key.point))
            await wait()
        } catch AuthError.cancelled {
            stage = .ready
        } catch {
            stage = .failed(error.localizedDescription)
        }
    }

    /// Asks every few seconds until the administrator admits this iPhone.
    func wait() async {
        let url = URL(string: "/v0/app/join/\(link.id)", relativeTo: link.base)!
        while case .waiting = stage {
            if let answer = try? await URLSession.shared.data(from: url),
                let s = try? JSONDecoder().decode(JoinState.self, from: answer.0)
            {
                switch s.status {
                case "admitted":
                    LearnerLink(base: link.base).save()
                    stage = .joined
                    return
                case "enrolled": break
                default:
                    stage = .failed("The invitation is \(s.status). Ask for a new link.")
                    return
                }
            }
            try? await Task.sleep(nanoseconds: 3_000_000_000)
        }
    }
}

/// The screen a join link opens.
struct LearnerJoinView: View {
    @StateObject private var join: LearnerJoin
    @State private var name = ""
    @Environment(\.dismiss) private var dismiss

    init(link: JoinLink) { _join = StateObject(wrappedValue: LearnerJoin(link: link)) }

    var body: some View {
        NavigationStack {
            ZStack {
                DotGrid()
                VStack(alignment: .leading, spacing: 18) {
                    Eyebrow(text: "Class 7L practice")
                    Text("Join your practice space").font(Theme.display(22)).foregroundStyle(Theme.ink)
                    content
                    Spacer()
                }
                .padding(20)
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch join.stage {
        case .ready:
            Text("Your practice will be saved online, so it's safe if this iPhone is lost. You'll make a passkey to sign in; nothing to remember.")
                .font(.footnote).foregroundStyle(Theme.muted)
            TextField("Your first name (optional)", text: $name)
                .textFieldStyle(.roundedBorder)
            Button { Task { await join.join(name: name) } } label: { learnerPrimary("Make my passkey") }
        case .working:
            ProgressView("Joining…")
        case let .waiting(code):
            Text("Show this code to the person who invited you:").font(.subheadline).foregroundStyle(Theme.ink)
            Text("\(code.prefix(3)) \(code.suffix(3))")
                .font(Theme.mono(40, .semibold)).foregroundStyle(Theme.ink)
            Text("Once they check it matches, this screen moves on by itself.")
                .font(.footnote).foregroundStyle(Theme.muted)
            ProgressView()
        case .joined:
            Label("You're in. Your practice so far is being saved online.", systemImage: "checkmark.circle")
                .foregroundStyle(Theme.good)
            Button { dismiss() } label: { learnerPrimary("Start practising") }
        case let .failed(why):
            Text(why).font(.footnote).foregroundStyle(Theme.kind(1))
            Button { join.stage = .ready } label: { learnerSecondary("Try again") }
        }
    }
}
