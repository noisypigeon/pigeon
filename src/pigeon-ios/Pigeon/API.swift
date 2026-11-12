// The control plane's /v0/app API, every call signed by the device's key; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §4.

import Foundation
import PigeonCore

struct APIError: Error, LocalizedError {
    let status: Int
    let message: String
    var errorDescription: String? { message.isEmpty ? "The control plane answered \(status)." : message }
}

struct API {
    static let host = "platform.pigeon.dev"
    static let base = URL(string: "https://\(host)")!

    let device: DeviceKey

    /// A signed request's answer, decoded.
    func call<T: Decodable>(
        _ method: String, _ path: String, body: [String: Any]? = nil, as type: T.Type
    ) async throws -> T {
        let data = try await send(method, path, body: body)
        return try JSONDecoder.pigeon.decode(T.self, from: data)
    }

    /// A signed request's raw answer.
    @discardableResult
    func send(_ method: String, _ path: String, body: [String: Any]? = nil) async throws -> Data {
        let bytes = try body.map { try JSONSerialization.data(withJSONObject: $0) } ?? Data()
        let at = Int(Date().timeIntervalSince1970)
        let pathOnly = path.split(separator: "?", maxSplits: 1).first.map(String.init) ?? path
        let signature = try device.sign(canonical(method: method, path: pathOnly, at: at, body: bytes))
        var request = URLRequest(url: URL(string: path, relativeTo: Self.base)!)
        request.httpMethod = method
        request.setValue(
            headerValue(device: device.id, at: at, signature: signature),
            forHTTPHeaderField: deviceHeader)
        if body != nil {
            request.httpBody = bytes
            request.setValue("application/json", forHTTPHeaderField: "content-type")
        }
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            throw APIError(status: status, message: String(decoding: data, as: UTF8.self))
        }
        return data
    }

    /// Redeems a link code: unsigned, since the device isn't linked until it answers.
    static func redeem(code: String, device: DeviceKey) async throws {
        let signature = try device.sign(linkMessage(code: code))
        let body: [String: Any] = [
            "code": code, "key": device.publicKey, "signature": base64url(signature),
        ]
        var request = URLRequest(url: base.appendingPathComponent("v0/app/session"))
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else {
            throw APIError(status: status, message: String(decoding: data, as: UTF8.self))
        }
    }
}
