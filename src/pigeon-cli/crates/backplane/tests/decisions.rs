//! Which ADRs govern a path; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §4.

use backplane::decisions::{governing, parse};

const ADR: &str = "## The factory's code\n\n* Date: 2026-11-10\n* Category: development\n\
* Governs: `src/pigeon-cli/crates/sentinel/**`, `tooling/ci/test-map.sh`\n\n## Context\n\n\
* Governs: `not/this/**`\n";

#[test]
fn an_adr_names_its_title_and_the_globs_in_its_header() {
    let d = parse("2026-11-10-x.md", ADR);
    assert_eq!(d.title, "The factory's code");
    assert_eq!(
        d.governs,
        [
            "src/pigeon-cli/crates/sentinel/**",
            "tooling/ci/test-map.sh"
        ],
        "only the header's line"
    );
    assert_eq!(
        parse("y.md", "## Old\n\n* Date: 2026-11-02\n").governs,
        Vec::<String>::new()
    );
}

#[test]
fn a_path_is_governed_by_the_adrs_whose_globs_match_it() {
    let all = [
        parse("a.md", ADR),
        parse(
            "b.md",
            "## B\n\n* Governs: `src/pigeon-cli/crates/*/src/lib.rs`\n",
        ),
        parse("c.md", "## C\n"),
    ];
    let files = |p: &str| {
        governing(&all, p)
            .iter()
            .map(|d| d.file.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        files("src/pigeon-cli/crates/sentinel/src/lib.rs"),
        ["a.md", "b.md"]
    );
    assert_eq!(files("./tooling/ci/test-map.sh"), ["a.md"]);
    assert_eq!(files("README.md"), Vec::<String>::new());
}
