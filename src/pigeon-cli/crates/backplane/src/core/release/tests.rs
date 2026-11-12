//! Lines, versions, tiers and the config's checks, from text and commits alone.

use super::config::{Config, Tier, Unit};
use super::lines::{
    Commit, Released, bumped, first_lines, line, next_lines, read_version, tier_text, tree_of,
};

fn commit(sha: &str, subject: &str, paths: &[&str]) -> Commit {
    Commit {
        sha: sha.repeat(40 / sha.len()),
        date: "2026-01-02".into(),
        subject: subject.into(),
        paths: paths.iter().map(|p| (*p).to_string()).collect(),
    }
}

fn unit(exclude: &[&str]) -> Unit {
    Unit {
        manifest: "crates/a/Cargo.toml".into(),
        features: String::new(),
        image: None,
        paths: vec!["crates/a/".into()],
        exclude: exclude.iter().map(|p| (*p).to_string()).collect(),
        cargo: None,
    }
}

#[test]
fn lines_link_prs_or_commits() {
    let c = Commit {
        sha: "0123456789abcdef0123456789abcdef01234567".into(),
        date: "2026-01-02".into(),
        subject: "feat: a (#12)".into(),
        paths: vec![],
    };
    assert_eq!(
        line("https://x", &c),
        "- feat: a ([#12](https://x/pull/12))"
    );
    let c = Commit {
        subject: "docs: b".into(),
        ..c
    };
    assert_eq!(
        line("https://x", &c),
        "- docs: b ([0123456](https://x/commit/0123456789abcdef0123456789abcdef01234567))"
    );
}

#[test]
fn reads_and_bumps_the_package_version_line() {
    let m = "[package]\nname = \"x\"\nversion = \"0.1.9\"\n";
    let (start, end, patch, mm) = read_version(m).expect("version");
    assert_eq!((&m[start..end], patch, mm.as_str()), ("0.1.9", 9, "0.1"));
    assert_eq!(
        bumped(m, start, end, "0.1.10"),
        "[package]\nname = \"x\"\nversion = \"0.1.10\"\n"
    );
    assert!(read_version("version.workspace = true\n").is_none());
    let yaml = "description: x\nversion: 1.2.3\n";
    let (s, e, p, mm) = read_version(yaml).expect("yaml");
    assert_eq!((&yaml[s..e], p, mm.as_str()), ("1.2.3", 3, "1.2"));
    let json = "{\n  \"x-cobot\": { \"version\": \"0.4.0\" }\n}\n";
    let (s, e, p, _) = read_version(json).expect("json");
    assert_eq!((&json[s..e], p), ("0.4.0", 0));
}

#[test]
fn next_lines_take_shipped_and_setting_commits_and_mark_the_dependency_change() {
    let range = [
        commit("a", "feat: shipped", &["crates/a/src/lib.rs"]),
        commit("b", "docs: readme only", &["crates/a/README.md"]),
        commit("c", "chore: bump deps", &["Cargo.lock"]),
        commit("d", "chore: lints", &["Cargo.toml"]),
    ];
    let prefixes = vec!["crates/a/".to_string()];
    let got = next_lines(
        "https://x",
        &unit(&["*.md"]),
        &range,
        &prefixes,
        &[range[3].sha.as_str()],
        true,
    );
    let short = |l: &str| l.split(" ([").next().unwrap_or_default().to_string();
    assert_eq!(
        got.iter().map(|l| short(l)).collect::<Vec<_>>(),
        [
            "- chore: lints",
            "- chore: bump deps (dependencies)",
            "- feat: shipped"
        ]
    );
    let none = next_lines(
        "https://x",
        &unit(&["*.md"]),
        &range[1..2],
        &prefixes,
        &[],
        false,
    );
    assert_eq!(none, Vec::<String>::new());
}

#[test]
fn a_first_release_falls_back_to_the_newest_commit() {
    let other = commit("e", "feat: elsewhere", &["crates/b/src/lib.rs"]);
    let got = first_lines(
        "https://x",
        &unit(&[]),
        std::slice::from_ref(&other),
        Some(&other),
    );
    assert_eq!(got, vec![line("https://x", &other)]);
    let own = commit("f", "feat: mine", &["crates/a/src/lib.rs"]);
    let got = first_lines(
        "https://x",
        &unit(&[]),
        &[own.clone(), other.clone()],
        Some(&other),
    );
    assert_eq!(got, vec![line("https://x", &own)]);
}

#[test]
fn a_paths_tier_adds_each_new_line_once_and_a_unit_tier_its_release() {
    let paths = Tier {
        file: "CHANGELOG.md".into(),
        paths: Some(vec!["crates/a/".into()]),
        unit: None,
        from: None,
    };
    let c = commit("a", "feat: x", &["crates/a/src/lib.rs"]);
    let once = tier_text(
        &paths,
        None,
        std::slice::from_ref(&c),
        "https://x",
        &[],
        "2026-01-03",
    );
    assert_eq!(
        once,
        format!(
            "# Changelog\n\n## 2026-01-02\n\n{}\n",
            line("https://x", &c)
        )
    );
    let twice = tier_text(&paths, Some(&once), &[c], "https://x", &[], "2026-01-03");
    assert_eq!(twice, once);
    let tier = Tier {
        file: "crates/a/CHANGELOG.md".into(),
        paths: None,
        unit: Some("a".into()),
        from: None,
    };
    let release = Released {
        unit: "a".into(),
        version: "0.1.1".into(),
        tag: "a/v0.1.1".into(),
        image: None,
        features: String::new(),
    };
    let text = tier_text(
        &tier,
        None,
        &[],
        "https://x",
        &[(release, vec!["- y".into()])],
        "2026-01-03",
    );
    assert_eq!(text, "# Changelog\n\n## v0.1.1 (2026-01-03)\n\n- y\n");
}

#[test]
fn cargo_tree_output_names_workspace_crates_and_their_paths() {
    let out = "a v0.1.0 (/tmp/t/ws/crates/a)\nb v0.2.0 (/tmp/t/ws/crates/b)\nserde v1.0.0\nb v0.2.0 (/tmp/t/ws/crates/b) (*)\n";
    let (crates, paths) = tree_of(out, " (/tmp/t/", "a");
    assert_eq!(
        crates.into_iter().collect::<Vec<_>>(),
        ["a (workspace)", "b (workspace)", "serde v1.0.0"]
    );
    assert_eq!(paths, ["ws/crates/b/"]);
}

#[test]
fn the_config_refuses_a_tier_with_both_or_neither_and_an_unknown_unit() {
    let base = "description: x\nrepository: https://x\nunits:\n  a:\n    manifest: crates/a/Cargo.toml\n    paths: [crates/a/]\n";
    let ok = format!("{base}tiers:\n  - file: CHANGELOG.md\n    unit: a\n");
    assert!(Config::parse(&ok).is_ok());
    let unknown = format!("{base}tiers:\n  - file: CHANGELOG.md\n    unit: b\n");
    assert!(Config::parse(&unknown).is_err_and(|e| e.contains("unknown unit `b`")));
    let neither = format!("{base}tiers:\n  - file: CHANGELOG.md\n");
    assert!(Config::parse(&neither).is_err_and(|e| e.contains("exactly one")));
    let retired = format!("{base}tiers: []\nretired: [../CHANGELOG.md]\n");
    assert!(Config::parse(&retired).is_err_and(|e| e.contains("under the root")));
}
