//! `cobot::release` against throwaway git repositories.

use std::fs;
use std::path::Path;
use std::process::Command;

use cobot::release::{self, Config, Outcome};
use tempfile::TempDir;

const CONFIG: &str = "description: test.
repository: https://x
units:
  a:
    manifest: a/Cargo.toml
    features: a
    image: img/a
    paths: [a/, shared/]
  b:
    manifest: b/Cargo.toml
    features: b
    image: img/b
    paths: [b/, shared/]
tiers:
  - { file: CHANGELOG.md, paths: [''] }
  - { file: a/CHANGELOG.md, unit: a }
  - { file: b/CHANGELOG.md, unit: b }
";

fn git(root: &Path, args: &[&str], date: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_AUTHOR_DATE", format!("{date}T12:00:00Z"))
        .env("GIT_COMMITTER_DATE", format!("{date}T12:00:00Z"))
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_string()
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn commit(root: &Path, subject: &str, files: &[&str], date: &str) -> String {
    for f in files {
        write(root, f, subject);
    }
    git(root, &["add", "-A"], date);
    git(root, &["commit", "-q", "-m", subject], date);
    git(root, &["rev-parse", "HEAD"], date)
}

fn repo() -> (TempDir, String) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"], "2026-01-01");
    write(
        root,
        "a/Cargo.toml",
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
    );
    write(
        root,
        "b/Cargo.toml",
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
    );
    let base = commit(root, "chore: init (#1)", &["shared/s"], "2026-01-01");
    (dir, base)
}

fn run(root: &Path, from: Option<&str>) -> Outcome {
    let config = Config::load(&{
        let p = root.join("releases.yaml");
        fs::write(&p, CONFIG).expect("write");
        p
    })
    .expect("config");
    release::run(root, &config, from, "HEAD", None, false).expect("release")
}

/// Commits the bot's output and tags each release, as the workflow does.
fn land(root: &Path, outcome: &Outcome, date: &str) -> String {
    git(root, &["add", "-A"], date);
    git(
        root,
        &["commit", "-q", "--allow-empty", "-m", "chore(release): x"],
        date,
    );
    for r in &outcome.released {
        git(root, &["tag", &r.tag], date);
    }
    git(root, &["rev-parse", "HEAD"], date)
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).expect("read")
}

fn released(o: &Outcome) -> Vec<String> {
    o.released.iter().map(|r| r.tag.clone()).collect()
}

#[test]
fn first_release_then_only_changed_units() {
    let (dir, _) = repo();
    let root = dir.path();

    let first = run(root, None);
    assert_eq!(released(&first), ["a/v0.1.0", "b/v0.1.0"]);
    assert_eq!(
        read(root, "CHANGELOG.md"),
        "# Changelog\n\n## 2026-01-01\n\n- chore: init ([#1](https://x/pull/1))\n"
    );
    assert_eq!(
        read(root, "a/CHANGELOG.md"),
        "# Changelog\n\n## v0.1.0 (2026-01-01)\n\n- chore: init ([#1](https://x/pull/1))\n"
    );
    let mut before = land(root, &first, "2026-01-01");

    let sha = commit(root, "fix: a only", &["a/src.rs"], "2026-01-02");
    let a_only = run(root, Some(&before));
    assert_eq!(released(&a_only), ["a/v0.1.1"]);
    assert!(read(root, "a/Cargo.toml").contains("version = \"0.1.1\"\n"));
    assert!(read(root, "b/Cargo.toml").contains("version = \"0.1.0\"\n"));
    assert!(read(root, "a/CHANGELOG.md").starts_with(&format!(
        "# Changelog\n\n## v0.1.1 (2026-01-02)\n\n- fix: a only ([{}](https://x/commit/{sha}))\n\n## v0.1.0",
        &sha[..7]
    )));
    before = land(root, &a_only, "2026-01-02");

    commit(root, "build: shared (#3)", &["shared/t"], "2026-01-03");
    let both = run(root, Some(&before));
    assert_eq!(released(&both), ["a/v0.1.2", "b/v0.1.1"]);
    before = land(root, &both, "2026-01-03");

    commit(root, "docs: readme (#4)", &["docs/readme"], "2026-01-03");
    let none = run(root, Some(&before));
    assert_eq!(released(&none), Vec::<String>::new());
    assert_eq!(none.written, [Path::new("CHANGELOG.md")]);
    assert!(read(root, "CHANGELOG.md").starts_with(
        "# Changelog\n\n## 2026-01-03\n\n- docs: readme ([#4](https://x/pull/4))\n- build: shared ([#3](https://x/pull/3))\n\n## 2026-01-02\n"
    ));

    let again = run(root, Some(&before));
    assert_eq!(released(&again), Vec::<String>::new());
    assert_eq!(again.written, Vec::<std::path::PathBuf>::new());
}

#[test]
fn dry_run_writes_nothing() {
    let (dir, _) = repo();
    let root = dir.path();
    let config_path = root.join("releases.yaml");
    fs::write(&config_path, CONFIG).expect("write");
    let config = Config::load(&config_path).expect("config");
    let outcome = release::run(root, &config, None, "HEAD", None, true).expect("release");
    assert_eq!(outcome.released.len(), 2);
    assert!(!root.join("CHANGELOG.md").exists());
}

#[test]
fn notes_hold_each_release_section() {
    let (dir, _) = repo();
    let root = dir.path();
    let config_path = root.join("releases.yaml");
    fs::write(&config_path, CONFIG).expect("write");
    let config = Config::load(&config_path).expect("config");
    let notes = root.join("notes");
    release::run(root, &config, None, "HEAD", Some(&notes), false).expect("release");
    assert_eq!(
        read(root, "notes/b.md"),
        "- chore: init ([#1](https://x/pull/1))\n"
    );
}

#[test]
fn invalid_config_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yaml");
    for bad in [
        CONFIG.replace(
            "{ file: b/CHANGELOG.md, unit: b }",
            "{ file: b/CHANGELOG.md, unit: c }",
        ),
        CONFIG.replace("unit: b }", "unit: b, paths: [b/] }"),
        CONFIG.replace("image: img/a", "image: img/a\n    extra: 1"),
    ] {
        fs::write(&path, bad).expect("write");
        assert!(Config::load(&path).is_err());
    }
}

#[test]
fn non_ascii_paths_release_their_unit() {
    let (dir, _) = repo();
    let root = dir.path();
    let first = run(root, None);
    land(root, &first, "2026-01-01");
    commit(
        root,
        "docs: caf\u{e9} (#2)",
        &["b/caf\u{e9}.md"],
        "2026-01-02",
    );
    assert_eq!(released(&run(root, None)), ["b/v0.1.1"]);
}

#[test]
fn default_range_starts_after_the_last_release_commit() {
    let (dir, _) = repo();
    let root = dir.path();
    let first = run(root, None);
    land(root, &first, "2026-01-01");
    commit(root, "docs: one (#2)", &["docs/one"], "2026-01-02");
    commit(root, "docs: two (#3)", &["docs/two"], "2026-01-02");
    fs::write(root.join("CHANGELOG.md"), "# Changelog\n").expect("write");
    let outcome = run(root, None);
    assert_eq!(released(&outcome), Vec::<String>::new());
    assert_eq!(
        read(root, "CHANGELOG.md"),
        "# Changelog\n\n## 2026-01-02\n\n- docs: two ([#3](https://x/pull/3))\n- docs: one ([#2](https://x/pull/2))\n"
    );
}

#[test]
fn tags_lists_every_tag_and_flags_the_newest_by_version() {
    let (dir, _) = repo();
    let root = dir.path();
    let config_path = root.join("releases.yaml");
    fs::write(&config_path, CONFIG).expect("write");
    let config = Config::load(&config_path).expect("config");
    assert_eq!(release::tags(root, &config).expect("tags"), []);
    for tag in ["a/v0.1.0", "a/v0.1.1", "a/v0.1.10", "b/v0.1.0"] {
        git(root, &["tag", tag], "2026-01-01");
    }
    let rows: Vec<(String, bool)> = release::tags(root, &config)
        .expect("tags")
        .into_iter()
        .map(|r| (r.tag, r.newest))
        .collect();
    assert_eq!(
        rows,
        [
            ("a/v0.1.10".to_string(), true),
            ("a/v0.1.1".to_string(), false),
            ("a/v0.1.0".to_string(), false),
            ("b/v0.1.0".to_string(), true),
        ]
    );
}

#[test]
fn a_unit_may_have_no_image() {
    let (dir, _) = repo();
    let root = dir.path();
    let path = root.join("releases.yaml");
    fs::write(&path, CONFIG.replace("    image: img/b\n", "")).expect("write");
    let config = Config::load(&path).expect("config");
    let outcome = release::run(root, &config, None, "HEAD", None, true).expect("release");
    let images: Vec<Option<String>> = outcome.released.iter().map(|r| r.image.clone()).collect();
    assert_eq!(images, [Some("img/a".to_string()), None]);
}

#[test]
fn tags_carry_each_units_changelog() {
    let (dir, _) = repo();
    let root = dir.path();
    let config_path = root.join("releases.yaml");
    fs::write(&config_path, CONFIG).expect("write");
    let config = Config::load(&config_path).expect("config");
    git(root, &["tag", "a/v0.1.0"], "2026-01-01");
    let rows = release::tags(root, &config).expect("tags");
    assert_eq!(rows[0].changelog.as_deref(), Some("a/CHANGELOG.md"));
}

#[test]
fn yaml_manifests_bump_in_place() {
    let (dir, _) = repo();
    let root = dir.path();
    write(
        root,
        "m/module.yaml",
        "version: 0.1.0\ndescription: A module.\n",
    );
    let yaml_config = format!("{CONFIG}  - {{ file: m/CHANGELOG.md, unit: m }}\n").replace(
        "tiers:\n",
        "  m:\n    manifest: m/module.yaml\n    paths: [m/]\ntiers:\n",
    );
    let config_path = root.join("releases.yaml");
    fs::write(&config_path, &yaml_config).expect("write");
    let config = Config::load(&config_path).expect("config");
    let first = release::run(root, &config, None, "HEAD", None, false).expect("first");
    land(root, &first, "2026-01-01");
    commit(root, "fix: m (#2)", &["m/main.tf"], "2026-01-02");
    let second = release::run(root, &config, None, "HEAD", None, false).expect("second");
    assert!(
        second.released.iter().any(|r| r.tag == "m/v0.1.1"),
        "{:?}",
        second.released
    );
    assert_eq!(
        read(root, "m/module.yaml"),
        "version: 0.1.1\ndescription: A module.\n"
    );
}

/// Runs `release` with `config`; `Err` carries the error text.
fn run_with(root: &Path, config: &str, dry_run: bool) -> Result<Outcome, String> {
    let p = root.join("releases.yaml");
    fs::write(&p, config).expect("write");
    let config = Config::load(&p).map_err(|e| e.to_string())?;
    release::run(root, &config, None, "HEAD", None, dry_run).map_err(|e| e.to_string())
}

fn moved(config: &str) -> String {
    config
        .replace(
            "{ file: a/CHANGELOG.md, unit: a }",
            "{ file: moved/a/CHANGELOG.md, unit: a, from: a/CHANGELOG.md }",
        )
        .replace(
            "{ file: CHANGELOG.md, paths: [''] }",
            "{ file: docs/CHANGELOG.md, paths: [''], from: CHANGELOG.md }",
        )
}

#[test]
fn a_moved_tier_keeps_its_history_and_the_bot_moves_its_file() {
    let (dir, _) = repo();
    let root = dir.path();
    let first = run(root, None);
    land(root, &first, "2026-01-01");
    commit(root, "feat: a change (#2)", &["a/x"], "2026-01-02");
    let second = run(root, None);
    land(root, &second, "2026-01-02");
    let unit_before = read(root, "a/CHANGELOG.md");
    let date_before = read(root, "CHANGELOG.md");

    commit(root, "chore: unrelated (#3)", &["shared/t"], "2026-01-03");
    let out = run_with(root, &moved(CONFIG), false).expect("release");
    let written: Vec<String> = out
        .written
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    for p in [
        "a/CHANGELOG.md",
        "moved/a/CHANGELOG.md",
        "CHANGELOG.md",
        "docs/CHANGELOG.md",
    ] {
        assert!(written.contains(&p.to_string()), "{p} in {written:?}");
    }
    assert!(!root.join("a/CHANGELOG.md").exists());
    assert!(!root.join("CHANGELOG.md").exists());
    let unit_after = read(root, "moved/a/CHANGELOG.md");
    assert!(
        unit_after.ends_with(&unit_before["# Changelog\n".len()..]),
        "{unit_after}"
    );
    assert!(
        unit_after.contains("v0.1.2"),
        "the shared change releases a once more: {unit_after}"
    );
    let date_after = read(root, "docs/CHANGELOG.md");
    for l in date_before.lines().filter(|l| l.starts_with("- ")) {
        assert!(date_after.contains(l), "kept `{l}` in {date_after}");
    }
    assert!(date_after.contains("unrelated"), "{date_after}");
    assert_eq!(
        date_after.matches("chore: init").count(),
        1,
        "not re-backfilled"
    );

    land(root, &out, "2026-01-03");
    let again = run_with(root, &moved(CONFIG), false).expect("rerun");
    assert_eq!(
        again.written,
        Vec::<std::path::PathBuf>::new(),
        "a gone `from` is ignored"
    );
}

#[test]
fn a_tier_in_both_places_fails() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root, None), "2026-01-01");
    write(root, "moved/a/CHANGELOG.md", "# Changelog\n");
    let err = run_with(root, &moved(CONFIG), false).expect_err("both exist");
    assert!(err.contains("a tier lives in one place"), "{err}");
}

#[test]
fn a_dry_run_moves_nothing() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root, None), "2026-01-01");
    let out = run_with(root, &moved(CONFIG), true).expect("dry run");
    assert!(
        out.written
            .iter()
            .any(|p| p.ends_with("moved/a/CHANGELOG.md"))
    );
    assert!(root.join("a/CHANGELOG.md").exists());
    assert!(!root.join("moved/a/CHANGELOG.md").exists());
}

#[test]
fn bad_froms_are_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yaml");
    for bad in [
        CONFIG.replace("unit: a }", "unit: a, from: a/CHANGELOG.md }"),
        CONFIG.replace("unit: b }", "unit: b, from: a/CHANGELOG.md }"),
        CONFIG
            .replace("unit: b }", "unit: b, from: x/CHANGELOG.md }")
            .replace("unit: a }", "unit: a, from: x/CHANGELOG.md }"),
    ] {
        fs::write(&path, bad).expect("write");
        assert!(Config::load(&path).is_err());
    }
}

#[test]
fn an_unreadable_tier_to_move_fails_and_keeps_it() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root, None), "2026-01-01");
    fs::write(root.join("a/CHANGELOG.md"), [0xff, 0xfe, 0x00]).expect("write");
    let err = run_with(root, &moved(CONFIG), false).expect_err("unreadable");
    assert!(err.contains("cannot read the tier to move"), "{err}");
    assert!(root.join("a/CHANGELOG.md").exists());
    assert!(!root.join("moved/a/CHANGELOG.md").exists());
}

#[test]
fn tags_carry_a_moved_tiers_previous_path() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root, None), "2026-01-01");
    let p = root.join("releases.yaml");
    fs::write(&p, moved(CONFIG)).expect("write");
    let rows = release::tags(root, &Config::load(&p).expect("config")).expect("tags");
    let a = rows.iter().find(|r| r.unit == "a").expect("a");
    assert_eq!(a.changelog.as_deref(), Some("moved/a/CHANGELOG.md"));
    assert_eq!(a.changelog_from.as_deref(), Some("a/CHANGELOG.md"));
    let b = rows.iter().find(|r| r.unit == "b").expect("b");
    assert_eq!(b.changelog_from, None);
}
