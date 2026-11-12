// Linking this iPhone through the Access-protected /app/link page, and proving the owner with the
// same passkey the pages use; see .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §2, §3.

import AuthenticationServices
import PigeonCore
import UIKit

/// The passkey's answer, as the control plane's prove step reads it.
struct PasskeyAnswer {
    let id: String
    let client: String
    let authenticator: String
    let signature: String

    var json: [String: Any] {
        ["id": id, "client": client, "authenticator": authenticator, "signature": signature]
    }
}

enum AuthError: Error, LocalizedError {
    case cancelled
    case noCode
    case badChallenge

    var errorDescription: String? {
        switch self {
        case .cancelled: return "Cancelled."
        case .noCode: return "The link page didn't hand back a code."
        case .badChallenge: return "The challenge couldn't be read."
        }
    }
}

@MainActor
final class Auth: NSObject, ASWebAuthenticationPresentationContextProviding,
    ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding
{
    private var session: ASWebAuthenticationSession?
    private var passkey: CheckedContinuation<PasskeyAnswer, Error>?

    private var window: ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first ?? ASPresentationAnchor()
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        window
    }

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        window
    }

    /// Opens the link page for this device and returns the one-time code it hands back.
    func link(device: DeviceKey) async throws -> String {
        var url = URLComponents(string: "https://\(API.host)/app/link")!
        url.queryItems = [
            URLQueryItem(name: "key", value: device.publicKey),
            URLQueryItem(name: "name", value: UIDevice.current.name),
        ]
        return try await withCheckedThrowingContinuation { done in
            let session = ASWebAuthenticationSession(
                url: url.url!, callbackURLScheme: "pigeon"
            ) { callback, error in
                if let error {
                    let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
                    done.resume(throwing: cancelled ? AuthError.cancelled : error)
                    return
                }
                let code = callback.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }?
                    .queryItems?.first { $0.name == "code" }?.value
                guard let code else {
                    done.resume(throwing: AuthError.noCode)
                    return
                }
                done.resume(returning: code)
            }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = false
            self.session = session
            session.start()
        }
    }

    /// Asks for the owner's passkey against `challenge`.
    func prove(_ challenge: PasskeyChallenge) async throws -> PasskeyAnswer {
        guard let bytes = fromBase64url(challenge.challenge) else { throw AuthError.badChallenge }
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: challenge.rp)
        let request = provider.createCredentialAssertionRequest(challenge: bytes)
        request.userVerificationPreference = .required
        request.allowedCredentials = challenge.ids.compactMap(fromBase64url).map {
            ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
        }
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = self
        controller.presentationContextProvider = self
        return try await withCheckedThrowingContinuation { done in
            self.passkey = done
            controller.performRequests()
        }
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        guard
            let a = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialAssertion
        else {
            passkey?.resume(throwing: AuthError.cancelled)
            passkey = nil
            return
        }
        passkey?.resume(
            returning: PasskeyAnswer(
                id: base64url(a.credentialID),
                client: base64url(a.rawClientDataJSON),
                authenticator: base64url(a.rawAuthenticatorData),
                signature: base64url(a.signature)))
        passkey = nil
    }

    func authorizationController(
        controller: ASAuthorizationController, didCompleteWithError error: Error
    ) {
        let cancelled = (error as? ASAuthorizationError)?.code == .canceled
        passkey?.resume(throwing: cancelled ? AuthError.cancelled : error)
        passkey = nil
    }
}
