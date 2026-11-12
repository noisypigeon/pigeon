// The device signature the control plane checks on every /v0/app call; it mirrors backplane's
// core::app::devices. See .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §2.

import CryptoKit
import Foundation

/// The request header carrying the signature.
public let deviceHeader = "X-Pigeon-Device"

/// Lowercase hex of `bytes`.
public func hex(_ bytes: some Sequence<UInt8>) -> String {
    bytes.map { String(format: "%02x", $0) }.joined()
}

/// The text a device signs for one request.
public func canonical(method: String, path: String, at: Int, body: Data) -> String {
    "pigeon-device-v1\n\(method.uppercased())\n\(path)\n\(at)\n\(hex(SHA256.hash(data: body)))"
}

/// The text a device signs to redeem a link code.
public func linkMessage(code: String) -> String {
    "pigeon-link-v1\n\(code)"
}

/// The header's value: the device, the time and the signature.
public func headerValue(device: String, at: Int, signature: Data) -> String {
    "id=\(device);t=\(at);s=\(base64url(signature))"
}

/// A key's fingerprint, the device's ID: the first 16 bytes of its raw point's SHA-256, hex.
public func fingerprint(x963 point: Data) -> String {
    hex(Array(SHA256.hash(data: point)).prefix(16))
}

/// Base64url without padding.
public func base64url(_ data: Data) -> String {
    data.base64EncodedString()
        .replacingOccurrences(of: "+", with: "-")
        .replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
}

/// Bytes from base64url, with or without padding.
public func fromBase64url(_ text: String) -> Data? {
    var s = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
    while s.count % 4 != 0 { s += "=" }
    return Data(base64Encoded: s)
}
