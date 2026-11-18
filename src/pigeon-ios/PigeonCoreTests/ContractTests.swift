import XCTest

@testable import PigeonCore

final class ContractTests: XCTestCase {
    private func view(_ json: String) throws -> ViewModel {
        try JSONDecoder().decode(ViewModel.self, from: Data(json.utf8))
    }

    private func value(_ json: String) throws -> Value {
        try JSONDecoder().decode(Value.self, from: Data(json.utf8))
    }

    func testList() throws {
        let v = try view(
            """
            {"view":"list","title":"Feed","next":"2","items":[{"id":"1","title":"A","needs":"owner",
            "open":{"label":"Open","tool":"change.status","arguments":{"number":1}}}]}
            """)
        guard case .list(let title, let items, let next) = v else { return XCTFail("not a list") }
        XCTAssertEqual(title, "Feed")
        XCTAssertEqual(next, "2")
        XCTAssertEqual(items[0].needs, .owner)
        XCTAssertEqual(items[0].open?.tool, "change.status")
        XCTAssertEqual(items[0].open?.arguments["number"], .number(1))
    }

    func testCard() throws {
        let v = try view(
            """
            {"view":"card","title":"C","body":"b","fields":[{"label":"n","value":{"kind":"count","n":3}}],
            "actions":[{"label":"Go","tool":"change.merge","style":"primary","proof":true}]}
            """)
        guard case .card(_, let body, let fields, let actions) = v else { return XCTFail("not a card") }
        XCTAssertEqual(body, "b")
        XCTAssertEqual(fields[0].value, .count(3))
        XCTAssertEqual(actions[0].style, .primary)
        XCTAssertTrue(actions[0].proof)
    }

    func testDetail() throws {
        let v = try view(
            """
            {"view":"detail","title":"D","sections":[{"heading":"H","fields":[],"text":"t"}],"actions":[]}
            """)
        guard case .detail(_, let sections, _) = v else { return XCTFail("not a detail") }
        XCTAssertEqual(sections[0].heading, "H")
        XCTAssertEqual(sections[0].text, "t")
    }

    func testForm() throws {
        let v = try view(
            """
            {"view":"form","title":"F","fields":[{"key":"k","label":"K","input":"secret","required":true}],
            "submit":{"label":"Send","tool":"x.y"}}
            """)
        guard case .form(_, let fields, let submit) = v else { return XCTFail("not a form") }
        XCTAssertEqual(fields[0].input, .secret)
        XCTAssertEqual(submit.tool, "x.y")
    }

    func testValueKinds() throws {
        XCTAssertEqual(try value(#"{"kind":"text","text":"a"}"#), .text("a"))
        XCTAssertEqual(try value(#"{"kind":"code","text":"a"}"#), .code("a"))
        XCTAssertEqual(try value(#"{"kind":"sha","text":"abc"}"#), .sha("abc"))
        XCTAssertEqual(try value(#"{"kind":"time","at":5}"#), .time(5))
        XCTAssertEqual(try value(#"{"kind":"duration","seconds":60}"#), .duration(60))
        XCTAssertEqual(try value(#"{"kind":"count","n":2}"#), .count(2))
        XCTAssertEqual(
            try value(#"{"kind":"link","text":"t","href":"https://x"}"#), .link(text: "t", href: "https://x"))
        XCTAssertEqual(
            try value(#"{"kind":"progress","stop":1,"of":3,"mark":"failed"}"#),
            .progress(stop: 1, of: 3, mark: .failed))
        XCTAssertEqual(
            try value(#"{"kind":"series","points":[{"label":"a","n":1}]}"#),
            .series([SeriesPoint(label: "a", n: 1)]))
    }

    func testUnknownValueKind() throws {
        XCTAssertEqual(try value(#"{"kind":"hologram","x":1}"#), .unknown)
    }

    private func outcome(_ json: String) throws -> MCPOutcome {
        try MCPOutcome.decode(Data(json.utf8))
    }

    func testOutcomeWithView() throws {
        let o = try outcome(
            """
            {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ok"}],
            "structuredContent":{"a":1},
            "_meta":{"dev.pigeon/view":{"view":"card","title":"T","fields":[],"actions":[]}}}}
            """)
        XCTAssertEqual(o.view?.title, "T")
        XCTAssertEqual(o.data?["a"], .number(1))
        XCTAssertNil(o.error)
        XCTAssertNil(o.pendingPath)
    }

    func testOutcomeError() throws {
        let o = try outcome(
            """
            {"jsonrpc":"2.0","id":1,"result":{"isError":true,
            "content":[{"type":"text","text":"bad"},{"type":"text","text":"worse"}]}}
            """)
        XCTAssertEqual(o.error, "bad\nworse")
        XCTAssertNil(o.view)
    }

    func testOutcomePendingLink() throws {
        let o = try outcome(
            """
            {"jsonrpc":"2.0","id":1,"result":{"content":[],
            "structuredContent":{"result":{"pending":{"link":"https://platform.pigeon.dev/approve/7?x=1"}}}}}
            """)
        XCTAssertEqual(o.pendingPath, "/approve/7?x=1")
    }

    func testOutcomePendingFromText() throws {
        let o = try outcome(
            """
            {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text",
            "text":"Approve at https://platform.pigeon.dev/mandate/9 now"}]}}
            """)
        XCTAssertEqual(o.pendingPath, "/mandate/9")
    }

    func testRPCError() {
        XCTAssertThrowsError(
            try outcome(#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"nope"}}"#)
        ) { XCTAssertEqual($0 as? MCPError, MCPError("nope")) }
    }

    func testRequestNames() {
        let r = MCPRequest.call(id: 2, tool: "change.status", arguments: ["number": .number(4)])
        let params = r["params"] as? [String: Any]
        XCTAssertEqual(params?["name"] as? String, "change_status")
        XCTAssertEqual(r["method"] as? String, "tools/call")
        let i = MCPRequest.initialize(id: 1)
        XCTAssertEqual((i["params"] as? [String: Any])?["protocolVersion"] as? String, "2025-06-18")
    }
}
