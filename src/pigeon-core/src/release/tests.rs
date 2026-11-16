//! Lines, versions, the config's checks, from text and commits alone.

use super::config::{Config, Unit};
use super::lines::{
    Commit, bumped, first_lines, line, next_lines, package_manifest, package_name, read_version,
    tag_rows, tree_of, workspace_dir,
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
        previous: None,
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
    let json = "{\n  \"x-pigeon\": { \"version\": \"0.4.0\" }\n}\n";
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
fn tag_rows_read_a_previous_units_tags() {
    let text = "description: x\nrepository: https://x\nchangelog: CHANGELOG.md\nunits:\n  new:\n    manifest: crates/a/Cargo.toml\n    paths: [crates/a/]\n    previous: old\n";
    let config = Config::parse(text).expect("config");
    let rows = tag_rows(
        &config,
        "new",
        &config.units["new"],
        vec![
            "new/v0.2.0".into(),
            "old/v0.1.9".into(),
            "other/v1.0.0".into(),
        ],
    );
    let got: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.tag.as_str(),
                r.version.as_str(),
                r.newest,
                r.changelog.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("new/v0.2.0", "0.2.0", true, "CHANGELOG.md"),
            ("old/v0.1.9", "0.1.9", false, "CHANGELOG.md")
        ]
    );
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
fn the_config_refuses_a_bad_changelog_or_previous() {
    let base = "description: x\nrepository: https://x\nunits:\n  a:\n    manifest: crates/a/Cargo.toml\n    paths: [crates/a/]\n";
    assert!(Config::parse(&format!("{base}changelog: CHANGELOG.md\n")).is_ok());
    for bad in ["NOTES.md", "../CHANGELOG.md", "/CHANGELOG.md"] {
        let text = format!("{base}changelog: {bad}\n");
        assert!(
            Config::parse(&text).is_err_and(|e| e.contains("is not a CHANGELOG.md under the root")),
            "{bad}"
        );
    }
    let both = format!("{base}changelog: CHANGELOG.md\nretired: [CHANGELOG.md]\n");
    assert!(Config::parse(&both).is_err_and(|e| e.contains("is both retired and the changelog")));
    let own = base.replace("    paths:", "    previous: a\n    paths:");
    let own = format!("{own}changelog: CHANGELOG.md\n");
    assert!(Config::parse(&own).is_err_and(|e| e.contains("is a current unit")));
    let two = format!(
        "{base}  b:\n    manifest: crates/b/Cargo.toml\n    paths: [crates/b/]\n    previous: z\nchangelog: CHANGELOG.md\n"
    )
    .replace("    paths: [crates/a/]\n", "    paths: [crates/a/]\n    previous: z\n");
    assert!(Config::parse(&two).is_err_and(|e| e.contains("names two units")));
    let current = format!(
        "{base}  b:\n    manifest: crates/b/Cargo.toml\n    paths: [crates/b/]\n    previous: a\nchangelog: CHANGELOG.md\n"
    );
    assert!(Config::parse(&current).is_err_and(|e| e.contains("is a current unit")));
    let defaulted =
        Config::parse(&format!("{base}tiers: []\n")).expect("tiers are read and ignored");
    assert_eq!(
        defaulted.changelog,
        std::path::PathBuf::from("CHANGELOG.md")
    );
}

#[test]
fn a_package_is_found_by_its_name_and_a_rename_shows_in_its_manifest() {
    let old = "[package]\nname = \"sentinel\"\nversion = \"0.1.16\"\n";
    let new = "[package]\nname = \"pigeon-sentinel\"\nversion = \"0.1.17\"\n";
    assert_eq!(package_name(old).as_deref(), Some("sentinel"));
    assert_eq!(package_name("[workspace]\nmembers = []\n"), None);
    assert_eq!(package_name("not toml ["), None);
    let manifests = [
        ("ws/Cargo.toml".to_string(), "[workspace]\n".to_string()),
        ("ws/crates/sentinel/Cargo.toml".to_string(), new.to_string()),
    ];
    assert_eq!(
        package_manifest(&manifests, "pigeon-sentinel"),
        Some("ws/crates/sentinel/Cargo.toml")
    );
    assert_eq!(package_manifest(&manifests, "sentinel"), None);
}

#[test]
fn the_workspace_is_the_shallowest_manifest_with_a_workspace_table() {
    let ws = "[workspace]\nmembers = [\"crates/*\"]\n";
    let pkg = "[package]\nname = \"a\"\n";
    let nested = vec![
        ("src/pigeon-cli/Cargo.toml".to_string(), ws.to_string()),
        (
            "src/pigeon-cli/crates/a/Cargo.toml".to_string(),
            pkg.to_string(),
        ),
    ];
    assert_eq!(workspace_dir(&nested).as_deref(), Some("src/pigeon-cli"));
    let root = vec![
        ("Cargo.toml".to_string(), ws.to_string()),
        (
            "src/pigeon-cli/crates/a/Cargo.toml".to_string(),
            pkg.to_string(),
        ),
        ("vendor/x/Cargo.toml".to_string(), ws.to_string()),
    ];
    assert_eq!(workspace_dir(&root).as_deref(), Some("."));
    let none = vec![("a/Cargo.toml".to_string(), pkg.to_string())];
    assert_eq!(workspace_dir(&none), None);
}
