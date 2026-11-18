// The app's state and what it asks the control plane, in one place the views read.

import CryptoKit
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
    @Published var tracker: Tracker?
    @Published var ops: Ops?
    @Published var instances: [InstanceEntry] = []

    private let auth = Auth()
    private var device: DeviceKey?
    private static let linkedKey = "linked"
    private var client: MCPClient?

    init() {
        linked = UserDefaults.standard.bool(forKey: Self.linkedKey)
    }

    private func api() throws -> API {
        if device == nil { device = try DeviceKey.load() }
        return API(device: device!)
    }

    /// The device door's MCP session; see the feed-view ADR's R10b amendment.
    func mcp() throws -> MCPClient {
        if let client { return client }
        let c = MCPClient(api: try api())
        client = c
        return c
    }

    /// Calls `name` on the device door and decodes its `result` as `T`.
    private func tool<T: Decodable>(
        _ name: String, _ args: [String: JSONValue] = [:], as type: T.Type
    ) async throws -> T {
        let o = try await mcp().call(name, args)
        if let e = o.error { throw APIError(status: 400, message: e) }
        guard let result = o.data?["result"] else {
            throw APIError(status: 0, message: "The answer had no result.")
        }
        return try JSONDecoder.pigeon.decode(T.self, from: JSONEncoder().encode(result))
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
            home = try await tool("home.show", as: Home.self)
            error = nil
        } catch let e as APIError where e.unlinks {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Reloads the tracker.
    func loadTracker() async {
        guard linked else { return }
        do {
            tracker = try await tool("tracker.show", as: Tracker.self)
        } catch let e as APIError where e.unlinks {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Reloads the operational insights over `range` days.
    func loadOps(range: Int) async {
        guard linked else { return }
        do {
            ops = try await tool("ops.show", ["range": .string(String(range))], as: Ops.self)
        } catch let e as APIError where e.unlinks {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Follows the feed until cancelled, reloading the tracker and Home on each batch.
    func follow() async {
        await loadTracker()
        var after = 9007199254740991
        do {
            after = try await tool(
                "feed.read", ["after": .string("9007199254740991"), "seconds": .string("0")],
                as: FeedPage.self
            ).last
        } catch {}
        var reloaded = Date.distantPast
        while !Task.isCancelled && linked {
            do {
                let batch = try await tool(
                    "feed.read", ["after": .string(String(after)), "seconds": .string("25")],
                    as: FeedPage.self)
                if batch.items.isEmpty {
                    if batch.last < after { after = batch.last }
                } else {
                    after = batch.last
                    if Date().timeIntervalSince(reloaded) >= 2 {
                        reloaded = Date()
                        await loadTracker()
                        await refresh()
                    }
                }
            } catch let e as APIError where e.unlinks {
                unlink(because: e)
                return
            } catch {
                if Task.isCancelled { return }
                try? await Task.sleep(nanoseconds: 30_000_000_000)
                await loadTracker()
            }
        }
    }

    /// One source's chart over `range` days.
    func stacks(metric: String, source: String, range: Int) async -> Stacks? {
        try? await tool(
            "insights.show",
            ["metric": .string(metric), "source": .string(source), "range": .string(String(range))],
            as: Stacks.self)
    }

    /// One change's sheet; nil when `change.status` can't be read.
    func changeDetail(_ number: Int) async -> ChangeDetail? {
        try? await tool(
            "change.status", ["number": .string(String(number))], as: ChangeDetail.self)
    }

    /// The proof page of approval `id`: the waiting page whose grant token's SHA-256 is `id`.
    func approvalPage(_ id: String) async -> String? {
        guard let list = try? await tool("approval.list", as: ApprovalPages.self) else { return nil }
        return list.waiting.first { w in
            guard w.path.hasPrefix("/approve/"),
                let token = w.path.components(separatedBy: "?grant=").dropFirst().first
            else { return false }
            return hex(SHA256.hash(data: Data(token.utf8))) == id
        }?.path
    }

    /// Reloads the instances: the platform, then the personal ones this account administers.
    func loadInstances() async {
        guard linked else { return }
        do {
            instances = try await tool("instances.list", as: InstanceList.self).instances
        } catch let e as APIError where e.unlinks {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Stands up a personal learner instance named `name` in the account `token` reaches,
    /// proved by the owner's passkey for that token; the token is sent once and never kept.
    func standUp(name: String, label: String, token: String) async -> Bool {
        await run {
            let token = token.trimmingCharacters(in: .whitespacesAndNewlines)
            let sha = hex(SHA256.hash(data: Data(token.utf8)))
            let answer = try await self.proof(["op": "instance:standup", "id": sha])
            try await self.api().send(
                "POST", "/v0/app/instances/standup",
                body: ["name": name, "label": label, "token": token, "passkey": answer.json])
            await self.loadInstances()
        }
    }

    /// Answers owner item `id`; an answer grants nothing, so it takes no passkey.
    func answer(item id: Int, with answer: String) async -> Bool {
        await run {
            _ = try await self.tool(
                "owner.answer", ["id": .string(String(id)), "answer": .string(answer)],
                as: JSONValue.self)
            await self.refresh()
        }
    }

    /// Loads the profile and its avatar; the first time, moves a local appearance into it.
    func loadProfile() async {
        guard linked else { return }
        do {
            let me = try await tool("profile.show", as: Me.self)
            let defaults = UserDefaults.standard
            var appearance = me.profile.appearance
            if !defaults.bool(forKey: "appearanceMigrated") {
                let local = defaults.string(forKey: "appearance").flatMap(Appearance.init(rawValue:))
                let r = Appearance.resolve(local: local, remote: me.profile.appearance)
                if let upload = r.upload {
                    _ = try await tool(
                        "profile.update", ["appearance": .string(upload.rawValue)], as: ProfileAnswer.self)
                }
                defaults.set(true, forKey: "appearanceMigrated")
                appearance = r.use
            }
            defaults.set(appearance.rawValue, forKey: "appearance")
            self.me = try await tool("profile.show", as: Me.self)
            do {
                avatar = fromBase64url(try await tool("avatar.show", as: AvatarAnswer.self).jpeg)
            } catch let e as APIError where e.message.contains("no avatar") {
                avatar = nil
            }
            error = nil
        } catch let e as APIError where e.unlinks {
            unlink(because: e)
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Renames the profile.
    func rename(_ name: String) async -> Bool {
        await patch(["name": .string(name)])
    }

    /// Sets the profile's appearance.
    func setAppearance(_ a: Appearance) async -> Bool {
        await patch(["appearance": .string(a.rawValue)])
    }

    private func patch(_ args: [String: JSONValue]) async -> Bool {
        await run {
            let answer = try await self.tool("profile.update", args, as: ProfileAnswer.self)
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
            _ = try await self.tool(
                "avatar.put", ["jpeg": .string(base64url(jpeg))], as: JSONValue.self)
            await self.loadProfile()
        }
    }

    /// Removes the avatar.
    func removeAvatar() async -> Bool {
        await run {
            _ = try await self.tool("avatar.remove", as: JSONValue.self)
            await self.loadProfile()
        }
    }

    /// Hands the control plane this device's APNs token.
    func registerPush(_ token: Data) async {
        guard linked else { return }
        _ = try? await tool("device.push", ["token": .string(hex(token))], as: JSONValue.self)
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
        tracker = nil
        ops = nil
        client = nil
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

/// `feed.read`'s answer, as `follow` reads it.
private struct FeedPage: Decodable {
    let last: Int
    let items: [JSONValue]
}

/// `approval.list`'s waiting pages.
private struct ApprovalPages: Decodable {
    struct Page: Decodable { let path: String }
    let waiting: [Page]
}

/// `avatar.show`'s answer: the photo, base64url.
private struct AvatarAnswer: Decodable {
    let jpeg: String
}
