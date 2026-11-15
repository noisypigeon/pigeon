// Joining a learner's standalone: the link, the match code backplane computes too, and the
// passkey's key read from a registration's attestation object.

import Foundation
import Testing

import PigeonCore

/// CBOR's head for `major` and a length or value under 256.
private func head(_ major: UInt8, _ n: Int) -> [UInt8] {
    n < 24 ? [major << 5 | UInt8(n)] : [major << 5 | 24, UInt8(n)]
}

private func bytes(_ b: [UInt8]) -> [UInt8] { head(2, b.count) + b }
private func text(_ s: String) -> [UInt8] { head(3, s.utf8.count) + Array(s.utf8) }

/// An attestation object, format `none`, for a P-256 key with coordinates `x` and `y`.
private func attestation(x: [UInt8], y: [UInt8], credential: [UInt8]) -> Data {
    let cose: [UInt8] =
        [0xa5]
        + [0x01, 0x02]  // 1 (kty): 2 (EC2)
        + [0x03, 0x26]  // 3 (alg): -7 (ES256)
        + [0x20, 0x01]  // -1 (crv): 1 (P-256)
        + [0x21] + bytes(x)  // -2: x
        + [0x22] + bytes(y)  // -3: y
    var auth = [UInt8](repeating: 0xaa, count: 32)
    auth += [0x45, 0, 0, 0, 0]
    auth += [UInt8](repeating: 0, count: 16)
    auth += [UInt8(credential.count >> 8), UInt8(credential.count & 0xff)] + credential
    auth += cose
    let object: [UInt8] =
        [0xa3] + text("fmt") + text("none") + text("attStmt") + [0xa0] + text("authData") + bytes(auth)
    return Data(object)
}

@Suite struct LearnerJoinTests {
    @Test func aJoinLinkNamesAWorkersDevHostAndAToken() throws {
        let token = String(repeating: "ab", count: 32)
        let ok = try #require(
            JoinLink(url: URL(string: "pigeon://join?base=https://pigeon-learner.example.workers.dev&token=\(token)")!))
        #expect(ok.base.host == "pigeon-learner.example.workers.dev")
        #expect(ok.id.count == 64)
        for bad in [
            "pigeon://join?base=http://x.workers.dev&token=\(token)",
            "pigeon://join?base=https://evil.example.com&token=\(token)",
            "pigeon://join?base=https://x.workers.dev&token=short",
            "pigeon://approval/1",
        ] {
            #expect(JoinLink(url: URL(string: bad)!) == nil, "\(bad)")
        }
    }

    @Test func aJoinLinkMayNameAStandalonesHostUnderThePlatformsZone() throws {
        let token = String(repeating: "ab", count: 32)
        let ok = try #require(
            JoinLink(url: URL(string: "pigeon://join?base=https://example.pigeon.dev&token=\(token)")!))
        #expect(ok.base.host == "example.pigeon.dev")
        for bad in [
            "pigeon://join?base=http://example.pigeon.dev&token=\(token)",
            "pigeon://join?base=https://a.example.pigeon.dev&token=\(token)",
            "pigeon://join?base=https://ab.pigeon.dev&token=\(token)",
            "pigeon://join?base=https://example.pigeon.dev.evil.example&token=\(token)",
            "pigeon://join?base=https://example.pigeon.dev/x&token=\(token)",
        ] {
            #expect(JoinLink(url: URL(string: bad)!) == nil, "\(bad)")
        }
    }

    @Test func aLabelHasBackplanesShape() {
        for good in ["example", "ex-ample2", String(repeating: "a", count: 32)] {
            #expect(isLabel(good), "\(good)")
        }
        for bad in ["ab", String(repeating: "a", count: 33), "Example", "1example", "example-", "ex--ample", "ex.ample"] {
            #expect(!isLabel(bad), "\(bad)")
        }
        #expect(standaloneLabel(host: "example.pigeon.dev") == "example")
        #expect(standaloneLabel(host: "example.workers.dev") == nil)
    }

    @Test func anInstanceEntryReadsItsHostWhenListedWithOne() throws {
        let json = #"{"id":"i1","name":"Practice","mode":"standalone","shape":"app","base_url":"https://example.pigeon.dev","features":["learner"],"status":"ready","host":"example.pigeon.dev"}"#
        let e = try JSONDecoder().decode(InstanceEntry.self, from: Data(json.utf8))
        #expect(e.host == "example.pigeon.dev")
        let old = #"{"name":"Practice","mode":"standalone","shape":"app","features":[]}"#
        #expect(try JSONDecoder().decode(InstanceEntry.self, from: Data(old.utf8)).host == nil)
    }

    @Test func theMatchCodeIsBackplanes() {
        let point = Data([4] + (1...64).map { UInt8($0) })
        #expect(matchCode(token: "t0k3n", publicKey: point) == "696816")
    }

    @Test func thePasskeysKeyIsReadFromItsAttestation() throws {
        let x = (0..<32).map { UInt8($0) }
        let y = (32..<64).map { UInt8($0) }
        let object = attestation(x: x, y: y, credential: [9, 9, 9])
        let read = try #require(attestedKey(attestationObject: object))
        #expect(read.point == Data([4] + x + y))
        #expect(read.authenticator.count == 32 + 5 + 16 + 2 + 3 + 77)
        #expect(spki(point: read.point).count == 91)
    }

    @Test func aBrokenAttestationReadsAsNothing() {
        #expect(attestedKey(attestationObject: Data([0xa0])) == nil)
        #expect(attestedKey(attestationObject: Data([0xff, 0x00])) == nil)
        let x = [UInt8](repeating: 1, count: 31)
        #expect(attestedKey(attestationObject: attestation(x: x, y: x, credential: [1])) == nil)
    }

    @Test func theJoinBodyIsWhatTheStandaloneReads() throws {
        let r = JoinRequest(
            token: "t", name: "n", deviceName: "d", passkeyID: "p", client: "c", authenticator: "a",
            spki: "s", deviceKey: "k", deviceSignature: "g")
        let json = try #require(
            try JSONSerialization.jsonObject(with: JSONEncoder().encode(r)) as? [String: Any])
        #expect(
            Set(json.keys) == [
                "token", "name", "device_name", "passkey_id", "client", "authenticator", "spki",
                "device_key", "device_signature",
            ])
    }
}
