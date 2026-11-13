// PigeonCore: the signature's text matches the control plane's, the answers decode, and the
// charts and words come out as the mockup shows.

import CryptoKit
import Foundation
import Testing

import PigeonCore

@Suite struct SigningTests {
    @Test func theCanonicalTextIsTheControlPlanes() {
        #expect(
            canonical(method: "get", path: "/v0/app/home", at: 1_800_000_005, body: Data())
                == "pigeon-device-v1\nGET\n/v0/app/home\n1800000005\n"
                + "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        #expect(
            canonical(
                method: "POST", path: "/v0/app/approvals/x", at: 1,
                body: Data(#"{"hold":false}"#.utf8))
                .hasSuffix("a34226fcab925a7c34a043907d0fb0a2732438d32fa2ad25d64d86b5b6c700ad"))
        #expect(linkMessage(code: "abc") == "pigeon-link-v1\nabc")
    }

    @Test func base64urlHasNoPaddingAndRoundTrips() {
        #expect(base64url(Data([0xfb, 0xff, 0x00])) == "-_8A")
        #expect(fromBase64url("-_8A") == Data([0xfb, 0xff, 0x00]))
        #expect(headerValue(device: "d", at: 5, signature: Data([0xfb, 0xff, 0x00])) == "id=d;t=5;s=-_8A")
    }

    @Test func aSignatureVerifiesWithTheKeysPoint() throws {
        let key = P256.Signing.PrivateKey()
        let text = canonical(method: "GET", path: "/v0/app/home", at: 7, body: Data())
        let sig = try key.signature(for: Data(text.utf8))
        #expect(key.publicKey.isValidSignature(sig, for: Data(text.utf8)))
        #expect(sig.rawRepresentation.count == 64)
        let id = fingerprint(x963: key.publicKey.x963Representation)
        #expect(id.count == 32)
        #expect(id == fingerprint(x963: key.publicKey.x963Representation))
    }
}

@Suite struct ModelTests {
    @Test func homeDecodesAsTheControlPlaneSendsIt() throws {
        let json = #"""
        {"insights":{"cost":{"metric":"cost","source":null,"range":1,"step":3600,"total":1.2,
        "estimate":true,"legend":[{"name":"buildkite","total":1.2}],
        "buckets":[{"at":3600,"parts":[{"name":"buildkite","value":1.2}]}]},"errors":null},
        "approvals":[{"id":"k","number":244,"operation":"change.approve","title":"t","expires":9}],
        "auto":{"id":1,"hours":4,"started":0,"expires":14400,"used":2,"state":"active","by":"o"}}
        """#
        let home = try JSONDecoder.pigeon.decode(Home.self, from: Data(json.utf8))
        #expect(home.insights.cost?.total == 1.2)
        #expect(home.insights.errors == nil)
        #expect(home.approvals.first?.number == 244)
        #expect(home.auto?.hours == 4)
    }
}

@Suite struct AskTests {
    @Test func aDetailDecodesWithAndWithoutAsks() throws {
        let plain = #"""
        {"id":"k","number":9,"operation":"change.approve","title":"t","branch":"change/9-x",
        "head":null,"state":"checked","scope":["a"],"plan":"p","checks":[],"tier":"code",
        "rule":"r","expires":9}
        """#
        let d = try JSONDecoder.pigeon.decode(ApprovalDetail.self, from: Data(plain.utf8))
        #expect(d.asked.isEmpty)
        let asked = #"""
        {"id":"k","number":9,"operation":"change.authorize","title":"t","branch":"change/9-x",
        "head":null,"state":"proposed","scope":["a"],"plan":"p","checks":[],"tier":"confirm",
        "rule":"r","expires":9,"asks":[{"key":"pm_verification","label":"pm-verification code",
        "help":"Proton Pass","chars":"lower","min":20,"max":40},
        {"key":"protonmail_verification","label":"code","chars":"hex","min":40,"max":40}]}
        """#
        let a = try JSONDecoder.pigeon.decode(ApprovalDetail.self, from: Data(asked.utf8))
        #expect(a.asked.map(\.key) == ["pm_verification", "protonmail_verification"])
        #expect(a.asked[1].help == nil)
    }

    @Test func anAnswerIsAdmittedOnlyInItsSetAndLength() {
        let lower = ApprovalAsk(key: "k", label: "l", help: nil, chars: "lower", min: 3, max: 5)
        #expect(lower.admits("abc"))
        #expect(!lower.admits("ab"))
        #expect(!lower.admits("abcdef"))
        #expect(!lower.admits("abC"))
        #expect(!lower.admits("ab é"))
        let hex = ApprovalAsk(key: "k", label: "l", help: nil, chars: "hex", min: 1, max: 9)
        #expect(hex.admits("0af9"))
        #expect(!hex.admits("0ag"))
        let token = ApprovalAsk(key: "k", label: "l", help: nil, chars: "token", min: 1, max: 99)
        #expect(token.admits("A.b:c=d+e/f-0_9"))
        #expect(!token.admits("a b"))
        #expect(!token.admits("a\"b"))
    }
}

@Suite struct ChartTests {
    let buckets = [
        Bucket(at: 0, parts: [Part(name: "cloudflare", value: 3), Part(name: "buildkite", value: 1)]),
        Bucket(at: 3_600, parts: [Part(name: "cloudflare", value: 2), Part(name: "buildkite", value: 0)]),
    ]

    @Test func segmentsStackFromTheBaselineAndScaleToTheTallest() {
        let s = segments(of: buckets[0], among: buckets, height: 100)
        #expect(s.map(\.name) == ["cloudflare", "buildkite"])
        #expect(s[0].height == 75 && s[0].offset == 0)
        #expect(s[1].height == 25 && s[1].offset == 75)
        let t = segments(of: buckets[1], among: buckets, height: 100)
        #expect(t.count == 1, "a part of 0 draws nothing")
        #expect(segments(of: Bucket(at: 0, parts: []), among: [], height: 10).isEmpty)
    }

    @Test func wordsAreTheMockups() {
        #expect(formatted(1.2, metric: "cost") == "$1.20")
        #expect(formatted(3.0, metric: "errors") == "3")
        #expect(formatted(-0.0, metric: "cost") == "$0.00", "never $-0.00")
        #expect(label(at: 14 * 3_600, step: 3_600, now: 15 * 3_600) == "14:00")
        #expect(label(at: 0, step: 86_400, now: 86_400 + 5) == "Yesterday")
        #expect(remaining(until: 10_000, now: 0) == "2h 46m")
        #expect(remaining(until: 600, now: 0) == "10m")
        #expect(sourceName("github") == "GitHub")
        #expect(tierBadge(nil) == "CODE")
    }

    @Test func aSourcesNoteIsReadAndOptional() throws {
        let json = """
            {"metric":"cost","source":"cloudflare","range":1,"step":3600,"total":0,
             "estimate":true,"legend":[],"buckets":[],"note":"Not collected yet."}
            """
        let s = try JSONDecoder.pigeon.decode(Stacks.self, from: Data(json.utf8))
        #expect(s.note == "Not collected yet.")
        let bare = json.replacingOccurrences(of: ",\"note\":\"Not collected yet.\"", with: "")
        #expect(try JSONDecoder.pigeon.decode(Stacks.self, from: Data(bare.utf8)).note == nil)
    }
}
