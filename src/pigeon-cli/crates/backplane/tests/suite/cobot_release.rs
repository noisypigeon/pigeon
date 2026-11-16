//! `backplane::cobot::release` against throwaway git repositories.

use backplane::cobot::release::LoadConfig;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use backplane::cobot::release::{self, Config, Outcome};
use tempfile::TempDir;

const CONFIG: &str = "description: test.
repository: https://x
changelog: CHANGELOG.md
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
    write(root, "releases.yaml", CONFIG);
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

fn config(root: &Path) -> Config {
    Config::load(&root.join("releases.yaml")).expect("config")
}

fn run_as(root: &Path, replay: bool, dry_run: bool) -> Outcome {
    release::run(
        root,
        &config(root),
        "releases.yaml",
        "HEAD",
        None,
        replay,
        dry_run,
    )
    .expect("release")
}

fn run(root: &Path) -> Outcome {
    run_as(root, false, false)
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

fn link(sha: &str, subject: &str) -> String {
    format!("- {subject} ([{}](https://x/commit/{sha}))", &sha[..7])
}

#[test]
fn first_release_then_only_changed_units() {
    let (dir, _) = repo();
    let root = dir.path();

    let first = run(root);
    assert_eq!(released(&first), ["a/v0.1.0", "b/v0.1.0"]);
    assert_eq!(
        read(root, "CHANGELOG.md"),
        "# Changelog\n\n## 2026-01-01\n\n### Released\n\n- a v0.1.0\n- b v0.1.0\n\n### Merged\n\n- chore: init ([#1](https://x/pull/1)) · a, b\n"
    );
    assert!(!root.join("a/CHANGELOG.md").exists());
    land(root, &first, "2026-01-01");

    let sha = commit(root, "fix: a only", &["a/src.rs"], "2026-01-02");
    let a_only = run(root);
    assert_eq!(released(&a_only), ["a/v0.1.1"]);
    assert!(read(root, "a/Cargo.toml").contains("version = \"0.1.1\"\n"));
    assert!(read(root, "b/Cargo.toml").contains("version = \"0.1.0\"\n"));
    assert!(!root.join("a/CHANGELOG.md").exists());
    assert!(read(root, "CHANGELOG.md").starts_with(&format!(
        "# Changelog\n\n## 2026-01-02\n\n### Released\n\n- a v0.1.1\n\n### Merged\n\n{} · a\n\n## 2026-01-01\n",
        link(&sha, "fix: a only")
    )));
    land(root, &a_only, "2026-01-02");

    commit(root, "build: shared (#3)", &["shared/t"], "2026-01-03");
    let both = run(root);
    assert_eq!(released(&both), ["a/v0.1.2", "b/v0.1.1"]);
    land(root, &both, "2026-01-03");

    commit(root, "docs: readme (#4)", &["docs/readme"], "2026-01-03");
    let none = run(root);
    assert_eq!(released(&none), Vec::<String>::new());
    assert_eq!(none.written, [Path::new("CHANGELOG.md")]);
    assert!(read(root, "CHANGELOG.md").starts_with(
        "# Changelog\n\n## 2026-01-03\n\n### Released\n\n- a v0.1.2\n- b v0.1.1\n\n### Merged\n\n- build: shared ([#3](https://x/pull/3)) · a, b\n- docs: readme ([#4](https://x/pull/4))\n\n## 2026-01-02\n"
    ));

    let again = run(root);
    assert_eq!(released(&again), Vec::<String>::new());
    assert_eq!(again.written, Vec::<PathBuf>::new());
}

#[test]
fn dry_run_writes_nothing() {
    let (dir, _) = repo();
    let root = dir.path();
    let outcome = run_as(root, false, true);
    assert_eq!(outcome.released.len(), 2);
    assert_eq!(outcome.written, [Path::new("CHANGELOG.md")]);
    assert!(!root.join("CHANGELOG.md").exists());
}

#[test]
fn notes_hold_each_release_section() {
    let (dir, _) = repo();
    let root = dir.path();
    let notes = root.join("notes");
    release::run(
        root,
        &config(root),
        "releases.yaml",
        "HEAD",
        Some(&notes),
        false,
        false,
    )
    .expect("release");
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
        CONFIG.replace("image: img/a", "image: img/a\n    previous: b"),
        CONFIG.replace("image: img/a", "image: img/a\n    extra: 1"),
        format!("{CONFIG}unknown: 1\n"),
    ] {
        fs::write(&path, bad).expect("write");
        assert!(Config::load(&path).is_err());
    }
}

#[test]
fn non_ascii_paths_release_their_unit() {
    let (dir, _) = repo();
    let root = dir.path();
    let first = run(root);
    land(root, &first, "2026-01-01");
    commit(
        root,
        "docs: caf\u{e9} (#2)",
        &["b/caf\u{e9}.md"],
        "2026-01-02",
    );
    assert_eq!(released(&run(root)), ["b/v0.1.1"]);
}

#[test]
fn tags_lists_every_tag_and_flags_the_newest_by_version() {
    let (dir, _) = repo();
    let root = dir.path();
    let config = config(root);
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
    write(
        root,
        "releases.yaml",
        &CONFIG.replace("    image: img/b\n", ""),
    );
    let outcome = run_as(root, false, true);
    let images: Vec<Option<String>> = outcome.released.iter().map(|r| r.image.clone()).collect();
    assert_eq!(images, [Some("img/a".to_string()), None]);
}

#[test]
fn tags_carry_each_units_changelog() {
    let (dir, _) = repo();
    let root = dir.path();
    git(root, &["tag", "a/v0.1.0"], "2026-01-01");
    let rows = release::tags(root, &config(root)).expect("tags");
    assert_eq!(rows[0].changelog, "CHANGELOG.md");
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
    write(
        root,
        "releases.yaml",
        &format!("{CONFIG}  m:\n    manifest: m/module.yaml\n    paths: [m/]\n"),
    );
    let first = run(root);
    land(root, &first, "2026-01-01");
    commit(root, "fix: m (#2)", &["m/main.tf"], "2026-01-02");
    let second = run(root);
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

/// Runs `release` with `config` written as the release config.
fn run_with(root: &Path, text: &str, dry_run: bool) -> Result<Outcome, String> {
    write(root, "releases.yaml", text);
    let config = Config::load(&root.join("releases.yaml")).map_err(|e| e.to_string())?;
    release::run(root, &config, "releases.yaml", "HEAD", None, false, dry_run)
        .map_err(|e| e.to_string())
}

fn retire_b(config: &str) -> String {
    let start = config.find("  b:\n").expect("unit b");
    format!("{}retired: [b/CHANGELOG.md]\n", &config[..start])
}

#[test]
fn the_bot_removes_a_retired_changelog_once() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root), "2026-01-01");
    write(root, "b/CHANGELOG.md", "# Changelog\n");

    let dry = run_with(root, &retire_b(CONFIG), true).expect("dry run");
    assert_eq!(dry.written, [Path::new("b/CHANGELOG.md")]);
    assert!(
        root.join("b/CHANGELOG.md").exists(),
        "a dry run removes nothing"
    );

    let out = run_with(root, &retire_b(CONFIG), false).expect("release");
    assert_eq!(out.written, [Path::new("b/CHANGELOG.md")]);
    assert!(!root.join("b/CHANGELOG.md").exists());
    land(root, &out, "2026-01-02");
    let again = run_with(root, &retire_b(CONFIG), false).expect("rerun");
    assert!(
        !again.written.iter().any(|p| p.ends_with("b/CHANGELOG.md")),
        "a gone retired file is ignored: {:?}",
        again.written
    );
}

#[test]
fn bad_retired_entries_are_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yaml");
    for bad in [
        format!("{CONFIG}retired: [b/NOTES.md]\n"),
        format!("{CONFIG}retired: [../x/CHANGELOG.md]\n"),
        format!("{CONFIG}retired: [/tmp/x/CHANGELOG.md]\n"),
        format!("{CONFIG}retired: [./x/CHANGELOG.md]\n"),
        format!("{CONFIG}retired: [CHANGELOG.md]\n"),
        retire_b(CONFIG).replace("[b/CHANGELOG.md]", "[x/CHANGELOG.md, x/CHANGELOG.md]"),
    ] {
        fs::write(&path, bad).expect("write");
        assert!(Config::load(&path).is_err());
    }
}

#[test]
fn replay_renders_history_and_cuts_nothing() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root), "2026-01-01");
    let sha = commit(root, "fix: a only", &["a/src.rs"], "2026-01-02");
    let out = run_as(root, true, false);
    assert_eq!(released(&out), Vec::<String>::new());
    assert_eq!(out.written, [Path::new("CHANGELOG.md")]);
    assert!(read(root, "a/Cargo.toml").contains("version = \"0.1.0\"\n"));
    assert_eq!(
        read(root, "CHANGELOG.md"),
        format!(
            "# Changelog\n\n## 2026-01-02\n\n### Merged\n\n{} · a\n\n## 2026-01-01\n\n### Released\n\n- a v0.1.0\n- b v0.1.0\n\n### Merged\n\n- chore: init ([#1](https://x/pull/1)) · a, b\n",
            link(&sha, "fix: a only")
        )
    );
}

#[test]
fn a_rerun_writes_nothing() {
    let (dir, _) = repo();
    let root = dir.path();
    let first = run_as(root, true, false);
    assert_eq!(first.written, [Path::new("CHANGELOG.md")]);
    let second = run_as(root, true, false);
    assert_eq!(second.written, Vec::<PathBuf>::new());
}

#[test]
fn a_changelog_is_the_same_from_any_checkout() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root), "2026-01-01");
    commit(root, "fix: a only", &["a/src.rs"], "2026-01-02");
    run_as(root, true, false);
    let copies = TempDir::new().expect("tempdir");
    let mut texts = vec![read(root, "CHANGELOG.md")];
    for name in ["one", "two"] {
        let copy = copies.path().join(name);
        git(
            copies.path(),
            &[
                "clone",
                "-q",
                &root.display().to_string(),
                &copy.display().to_string(),
            ],
            "2026-01-05",
        );
        run_as(&copy, true, false);
        texts.push(read(&copy, "CHANGELOG.md"));
    }
    assert_eq!(texts[0], texts[1]);
    assert_eq!(texts[1], texts[2]);
}

#[test]
fn lines_carry_units_as_named_when_they_merged() {
    let (dir, _) = repo();
    let root = dir.path();
    let one = commit(root, "feat: one", &["a/x"], "2026-01-02");
    write(
        root,
        "releases.yaml",
        &CONFIG.replace("  a:\n", "  alpha:\n    previous: a\n"),
    );
    let two = commit(root, "feat: two", &["a/y"], "2026-01-03");
    run_as(root, true, false);
    assert_eq!(
        read(root, "CHANGELOG.md"),
        format!(
            "# Changelog\n\n## 2026-01-03\n\n### Merged\n\n{} · alpha\n- units renamed · a → alpha\n\n## 2026-01-02\n\n### Merged\n\n{} · a\n\n## 2026-01-01\n\n### Merged\n\n- chore: init ([#1](https://x/pull/1)) · a, b\n",
            link(&two, "feat: two"),
            link(&one, "feat: one")
        )
    );
}

#[test]
fn a_renamed_unit_continues_its_previous_tags() {
    let (dir, _) = repo();
    let root = dir.path();
    land(root, &run(root), "2026-01-01");
    write(
        root,
        "releases.yaml",
        &CONFIG.replace("  a:\n", "  alpha:\n    previous: a\n"),
    );
    commit(root, "fix: under a", &["a/z"], "2026-01-02");
    let rows = |root: &Path| -> Vec<(String, String, bool)> {
        release::tags(root, &config(root))
            .expect("tags")
            .into_iter()
            .filter(|r| r.unit == "alpha")
            .map(|r| (r.tag, r.version, r.newest))
            .collect()
    };
    assert_eq!(rows(root), [("a/v0.1.0".into(), "0.1.0".into(), true)]);
    let out = run(root);
    assert_eq!(released(&out), ["alpha/v0.1.1"]);
    land(root, &out, "2026-01-02");
    assert_eq!(
        rows(root),
        [
            ("alpha/v0.1.1".into(), "0.1.1".into(), true),
            ("a/v0.1.0".into(), "0.1.0".into(), false),
        ]
    );
}

#[test]
fn a_commit_before_the_config_has_no_units() {
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
    let early = commit(root, "feat: early", &["a/x"], "2026-01-01");
    write(root, "releases.yaml", CONFIG);
    let added = commit(root, "chore: config", &[], "2026-01-02");
    run_as(root, true, false);
    assert_eq!(
        read(root, "CHANGELOG.md"),
        format!(
            "# Changelog\n\n## 2026-01-02\n\n### Merged\n\n{}\n\n## 2026-01-01\n\n### Merged\n\n{}\n",
            link(&added, "chore: config"),
            link(&early, "feat: early")
        )
    );
}

#[test]
fn bad_changelog_paths_are_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yaml");
    for bad in ["a/NOTES.md", "../CHANGELOG.md"] {
        fs::write(
            &path,
            CONFIG.replace("changelog: CHANGELOG.md", &format!("changelog: {bad}")),
        )
        .expect("write");
        assert!(Config::load(&path).is_err(), "{bad}");
    }
}
