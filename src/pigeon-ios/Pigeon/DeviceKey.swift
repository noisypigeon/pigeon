// The device's key: P-256 in the Secure Enclave, its handle kept in the Keychain; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §2.

import CryptoKit
import Foundation
import PigeonCore
import Security

enum DeviceKeyError: Error {
    case keychain(OSStatus)
}

struct DeviceKey {
    let key: SecureEnclave.P256.Signing.PrivateKey

    private static let account = "dev.pigeon.app.device-key"

    /// The device's key, made on first use.
    static func load() throws -> DeviceKey {
        if let data = try read() {
            return DeviceKey(key: try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: data))
        }
        let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, .privateKeyUsage, nil)!
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        try write(key.dataRepresentation)
        return DeviceKey(key: key)
    }

    /// The public key's raw point, base64url, as the link page and the control plane read it.
    var publicKey: String { base64url(key.publicKey.x963Representation) }

    /// The device's ID: its key's fingerprint.
    var id: String { fingerprint(x963: key.publicKey.x963Representation) }

    /// `text` signed, raw r‖s.
    func sign(_ text: String) throws -> Data {
        try key.signature(for: Data(text.utf8)).rawRepresentation
    }

    private static func read() throws -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
        ]
        var out: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw DeviceKeyError.keychain(status) }
        return out as? Data
    }

    private static func write(_ data: Data) throws {
        let item: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: account,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecValueData as String: data,
        ]
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw DeviceKeyError.keychain(status) }
    }
}
