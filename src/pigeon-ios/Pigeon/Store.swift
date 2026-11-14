// The app's state and what it asks the control plane, in one place the views read.

import Foundation
import PigeonCore
import SwiftUI

@MainActor
final class Store: ObservableObject {
    @Published var linked: Bool
    @Published var home: Home?
    @Published var error: String?
    @Published var busy = false
    @Published var me: Me?
    @Published var avatar: Data?

    private let auth = Auth()
    private var device: DeviceKey?
    private static let linkedKey = "linked"

    init() {
        linked = UserDefaults.standard.bool(forKey: Self.linkedKey)
    }

    private func api() throws -> API {
        if device == nil { device = try DeviceKey.load() }
        return API(device: device!)
    }

    /// Links this iPhone: the owner proves it on the link page, then the code is redeemed.
    func link() async {
        await run {
            let device = try DeviceKey.load()
            self.device = device
            let code = try await self.auth.link(device: device)
            try await API.redeem(code: code, device: device)
            UserDefaults.standard.set(true, forKey: Self.linkedKey)
            self.linked = true
            await self.refresh()
        }
    }

    /// Reloads Home.
    func refresh() async {
        guard linked else { return }
        do {
            home = try await api().call("GET", "/v0/app/home", as: Home.self)
            error = nil
        } catch let e as APIError where e.status == 401 {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// One source's chart over `range` days.
    func stacks(metric: String, source: String, range: Int) async -> Stacks? {
        try? await api().call(
            "GET", "/v0/app/insights?metric=\(metric)&source=\(source)&range=\(range)",
            as: Stacks.self)
    }

    /// One approval's card.
    func detail(_ id: String) async throws -> ApprovalDetail {
        try await api().call("GET", "/v0/app/approvals/\(id)", as: ApprovalDetail.self)
    }

    /// Approves `id` with the owner's passkey; `hold` keeps the merge, and `answers` answer
    /// the plan's asks.
    func approve(_ id: String, hold: Bool, answers: [String: String] = [:]) async -> Bool {
        await run {
            let answer = try await self.proof(["id": id])
            var body: [String: Any] = ["passkey": answer.json, "hold": hold]
            if !answers.isEmpty { body["answers"] = answers }
            try await self.api().send("POST", "/v0/app/approvals/\(id)", body: body)
            await self.refresh()
        }
    }

    /// Starts Auto for `hours` with the owner's passkey.
    func startAuto(hours: Int) async -> Bool {
        await run {
            let answer = try await self.proof(["op": "auto"])
            try await self.api().send(
                "POST", "/v0/app/auto", body: ["passkey": answer.json, "hours": hours])
            await self.refresh()
        }
    }

    /// Ends Auto early with the owner's passkey.
    func endAuto() async -> Bool {
        await run {
            let answer = try await self.proof(["op": "auto-end"])
            try await self.api().send("DELETE", "/v0/app/auto", body: ["passkey": answer.json])
            await self.refresh()
        }
    }

    /// Loads the profile and its avatar; the first time, moves a local appearance into it.
    func loadProfile() async {
        guard linked else { return }
        do {
            let api = try api()
            let me = try await api.call("GET", "/v0/app/profile", as: Me.self)
            let defaults = UserDefaults.standard
            var appearance = me.profile.appearance
            if !defaults.bool(forKey: "appearanceMigrated") {
                let local = defaults.string(forKey: "appearance").flatMap(Appearance.init(rawValue:))
                let r = Appearance.resolve(local: local, remote: me.profile.appearance)
                if let upload = r.upload {
                    try await api.send("PATCH", "/v0/app/profile", body: ["appearance": upload.rawValue])
                }
                defaults.set(true, forKey: "appearanceMigrated")
                appearance = r.use
            }
            defaults.set(appearance.rawValue, forKey: "appearance")
            self.me = try await api.call("GET", "/v0/app/profile", as: Me.self)
            do {
                avatar = try await api.send("GET", "/v0/app/profile/avatar")
            } catch let e as APIError where e.status == 404 {
                avatar = nil
            }
            error = nil
        } catch let e as APIError where e.status == 401 {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Renames the profile.
    func rename(_ name: String) async -> Bool {
        await patch(["name": name])
    }

    /// Sets the profile's appearance.
    func setAppearance(_ a: Appearance) async -> Bool {
        await patch(["appearance": a.rawValue])
    }

    private func patch(_ body: [String: Any]) async -> Bool {
        await run {
            let answer = try await self.api().call(
                "PATCH", "/v0/app/profile", body: body, as: ProfileAnswer.self)
            if let me = self.me {
                self.me = Me(
                    account: me.account, profile: answer.profile, passkeys: me.passkeys,
                    devices: me.devices)
            }
            UserDefaults.standard.set(answer.profile.appearance.rawValue, forKey: "appearance")
        }
    }

    /// Sets the avatar from JPEG bytes.
    func setAvatar(jpeg: Data) async -> Bool {
        await run {
            try await self.api().send(
                "PUT", "/v0/app/profile/avatar", body: ["jpeg": base64url(jpeg)])
            await self.loadProfile()
        }
    }

    /// Removes the avatar.
    func removeAvatar() async -> Bool {
        await run {
            try await self.api().send("DELETE", "/v0/app/profile/avatar")
            await self.loadProfile()
        }
    }

    /// Unlinks device `id` with a passkey; unlinking this iPhone signs it out.
    func unlinkDevice(_ id: String) async -> Bool {
        let current = me?.devices.first { $0.id == id }?.current ?? false
        let done = await run {
            let answer = try await self.proof(["op": "unlink", "id": id])
            try await self.api().send(
                "DELETE", "/v0/app/devices/\(id)", body: ["passkey": answer.json])
            if !current { await self.loadProfile() }
        }
        // After `run`, which clears `error`, so the sign-out's reason stays shown.
        if done && current {
            unlink(because: APIError(status: 0, message: "This iPhone was unlinked."))
        }
        return done
    }

    /// Removes passkey `id` with a passkey.
    func removePasskey(_ id: String) async -> Bool {
        await run {
            let answer = try await self.proof(["op": "passkey:remove", "id": id])
            try await self.api().send(
                "DELETE", "/v0/app/passkeys/\(id)", body: ["passkey": answer.json])
            await self.loadProfile()
        }
    }

    /// Hands the control plane this device's APNs token.
    func registerPush(_ token: Data) async {
        guard linked else { return }
        _ = try? await api().send("PUT", "/v0/app/device/push", body: ["token": hex(token)])
    }

    private func proof(_ body: [String: Any]) async throws -> PasskeyAnswer {
        let challenge = try await api().call(
            "POST", "/v0/app/challenge", body: body, as: PasskeyChallenge.self)
        return try await auth.prove(challenge)
    }

    private func unlink(because e: APIError) {
        UserDefaults.standard.set(false, forKey: Self.linkedKey)
        linked = false
        home = nil
        me = nil
        avatar = nil
        error = "This iPhone needs linking again: \(e.localizedDescription)"
    }

    @discardableResult
    private func run(_ work: @escaping () async throws -> Void) async -> Bool {
        busy = true
        defer { busy = false }
        do {
            try await work()
            error = nil
            return true
        } catch AuthError.cancelled {
            return false
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }
}
