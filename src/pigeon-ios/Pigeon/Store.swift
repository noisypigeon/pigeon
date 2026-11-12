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

    /// Approves `id` with the owner's passkey; `hold` keeps the merge.
    func approve(_ id: String, hold: Bool) async -> Bool {
        await run {
            let answer = try await self.proof(["id": id])
            try await self.api().send(
                "POST", "/v0/app/approvals/\(id)",
                body: ["passkey": answer.json, "hold": hold])
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
