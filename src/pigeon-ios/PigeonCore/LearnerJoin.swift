// Joining a learner's standalone from its invitation link: the link, the match code both sides
// compare, and the passkey's key read from its attestation, as the standalone's join checks
// them; see .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md, its
// 2026-11-15 amendment, §7.

import CryptoKit
import Foundation

/// `pigeon://join?base=<https://….workers.dev>&token=<64 hex>`.
public struct JoinLink: Equatable, Sendable, Identifiable {
    public let base: URL
    public let token: String

    public init?(url: URL) {
        guard url.scheme == "pigeon", url.host == "join",
            let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
            let b = items.first(where: { $0.name == "base" })?.value,
            let token = items.first(where: { $0.name == "token" })?.value,
            let base = URL(string: b), base.scheme == "https",
            let host = base.host, host.hasSuffix(".workers.dev"), base.path.isEmpty || base.path == "/",
            token.count == 64, token.allSatisfy(\.isHexDigit)
        else { return nil }
        self.base = base
        self.token = token
    }

    /// The invitation's ID: the token's SHA-256, hex.
    public var id: String { hex(SHA256.hash(data: Data(token.utf8))) }
}

/// The text the joining device signs.
public func joinMessage(token: String) -> String { "pigeon-join-v1\n\(token)" }

/// The six digits the invitee and an administrator compare: SHA-256 of the token, a zero byte
/// and the passkey's point, its first four bytes modulo a million. As backplane's `match_code`.
public func matchCode(token: String, publicKey: Data) -> String {
    var data = Data(token.utf8)
    data.append(0)
    data.append(publicKey)
    let d = Array(SHA256.hash(data: data))
    let n = (UInt32(d[0]) << 24) | (UInt32(d[1]) << 16) | (UInt32(d[2]) << 8) | UInt32(d[3])
    return String(format: "%06d", n % 1_000_000)
}

/// A P-256 point as a `SubjectPublicKeyInfo`, as the standalone reads a passkey's key.
public func spki(point: Data) -> Data {
    Data([
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
        0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ]) + point
}

/// A CBOR value, as much of it as an attestation object needs.
public indirect enum CBOR: Equatable, Sendable {
    case int(Int)
    case bytes(Data)
    case text(String)
    case array([CBOR])
    case map([(CBOR, CBOR)])
    case simple(UInt8)

    public static func == (a: CBOR, b: CBOR) -> Bool {
        switch (a, b) {
        case let (.int(x), .int(y)): return x == y
        case let (.bytes(x), .bytes(y)): return x == y
        case let (.text(x), .text(y)): return x == y
        case let (.array(x), .array(y)): return x == y
        case let (.map(x), .map(y)):
            return x.count == y.count && zip(x, y).allSatisfy { $0.0 == $1.0 && $0.1 == $1.1 }
        case let (.simple(x), .simple(y)): return x == y
        default: return false
        }
    }

    public subscript(key: CBOR) -> CBOR? {
        guard case let .map(pairs) = self else { return nil }
        return pairs.first { $0.0 == key }?.1
    }
}

/// Reads one CBOR value from `bytes` at `at`, moving `at` past it.
public func readCBOR(_ bytes: [UInt8], _ at: inout Int, depth: Int = 0) -> CBOR? {
    guard at < bytes.count, depth < 16 else { return nil }
    let head = bytes[at]
    at += 1
    let major = head >> 5
    let info = head & 0x1f
    var n: UInt64
    switch info {
    case 0..<24: n = UInt64(info)
    case 24, 25, 26, 27:
        let size = 1 << Int(info - 24)
        guard at + size <= bytes.count else { return nil }
        n = bytes[at..<(at + size)].reduce(0) { ($0 << 8) | UInt64($1) }
        at += size
    default: return nil
    }
    switch major {
    case 0: return n <= UInt64(Int.max) ? .int(Int(n)) : nil
    case 1: return n < UInt64(Int.max) ? .int(-1 - Int(n)) : nil
    case 2, 3:
        guard n <= UInt64(bytes.count - at) else { return nil }
        let slice = Data(bytes[at..<(at + Int(n))])
        at += Int(n)
        if major == 2 { return .bytes(slice) }
        return String(data: slice, encoding: .utf8).map(CBOR.text)
    case 4:
        guard n <= UInt64(bytes.count) else { return nil }
        var items: [CBOR] = []
        for _ in 0..<Int(n) {
            guard let v = readCBOR(bytes, &at, depth: depth + 1) else { return nil }
            items.append(v)
        }
        return .array(items)
    case 5:
        guard n <= UInt64(bytes.count) else { return nil }
        var pairs: [(CBOR, CBOR)] = []
        for _ in 0..<Int(n) {
            guard let k = readCBOR(bytes, &at, depth: depth + 1),
                let v = readCBOR(bytes, &at, depth: depth + 1)
            else { return nil }
            pairs.append((k, v))
        }
        return .map(pairs)
    case 7: return info < 24 ? .simple(info) : nil
    default: return nil
    }
}

/// What a registration's attestation object carries that the standalone needs: its
/// authenticator data, and the passkey's P-256 point (`0x04‖x‖y`) from its COSE key.
public func attestedKey(attestationObject: Data) -> (authenticator: Data, point: Data)? {
    var at = 0
    let bytes = [UInt8](attestationObject)
    guard let object = readCBOR(bytes, &at), case let .bytes(auth)? = object[.text("authData")] else {
        return nil
    }
    let a = [UInt8](auth)
    // rpIdHash 32, flags 1, counter 4, then the attested credential: AAGUID 16, length 2, ID.
    guard a.count > 55, a[32] & 0x40 != 0 else { return nil }
    let idLength = Int(a[53]) << 8 | Int(a[54])
    var keyAt = 55 + idLength
    guard keyAt < a.count, let cose = readCBOR(a, &keyAt) else { return nil }
    guard cose[.int(1)] == .int(2), cose[.int(3)] == .int(-7), cose[.int(-1)] == .int(1),
        case let .bytes(x)? = cose[.int(-2)], case let .bytes(y)? = cose[.int(-3)],
        x.count == 32, y.count == 32
    else { return nil }
    return (auth, Data([0x04]) + x + y)
}

/// What `POST /v0/app/join` takes.
public struct JoinRequest: Codable, Equatable, Sendable {
    public let token: String
    public let name: String
    public let device_name: String
    public let passkey_id: String
    public let client: String
    public let authenticator: String
    public let spki: String
    public let device_key: String
    public let device_signature: String

    public init(
        token: String, name: String, deviceName: String, passkeyID: String, client: String,
        authenticator: String, spki: String, deviceKey: String, deviceSignature: String
    ) {
        self.token = token
        self.name = name
        self.device_name = deviceName
        self.passkey_id = passkeyID
        self.client = client
        self.authenticator = authenticator
        self.spki = spki
        self.device_key = deviceKey
        self.device_signature = deviceSignature
    }
}

/// What `POST /v0/app/join` answers.
public struct Joined: Codable, Equatable, Sendable {
    public let id: String
    public let match_code: String?
    public let status: String
}

/// An instance as `GET /v0/app/instances` lists it.
public struct InstanceEntry: Codable, Equatable, Sendable, Identifiable {
    public let id: String?
    public let name: String
    public let mode: String
    public let shape: String
    public let base_url: String?
    public let accent: String?
    public let features: [String]
    public let status: String?
    public let why: String?

    public init(
        id: String?, name: String, mode: String, shape: String, base_url: String?, accent: String?,
        features: [String], status: String?, why: String?
    ) {
        self.id = id
        self.name = name
        self.mode = mode
        self.shape = shape
        self.base_url = base_url
        self.accent = accent
        self.features = features
        self.status = status
        self.why = why
    }

    public var key: String { id ?? name }
    public var learner: Bool { features.contains("learner") }
    public var ready: Bool { (status ?? "ready") == "ready" && base_url != nil }
}

/// `GET /v0/app/instances`'s answer.
public struct InstanceList: Codable, Equatable, Sendable {
    public let instances: [InstanceEntry]
}

/// One invitation as an administrator's view of the members lists it.
public struct InvitationRow: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let email: String
    public let role: String
    public let status: String
    public let expires: Int
    public let match_code: String?
}

/// `GET /v0/app/members`'s answer, as far as the learner's instance uses it.
public struct MembersView: Codable, Equatable, Sendable {
    public let invitations: [InvitationRow]
}

/// One question's record, as the instance keeps it.
public struct QuestionTally: Codable, Equatable, Sendable {
    public let seen: Int
    public let right: Int
    public let wrong: Int
    public let last: Int
}

/// `GET /v0/app/learner/attempts`'s answer.
public struct AttemptList: Codable, Equatable, Sendable {
    public let attempts: [Attempt]
}
