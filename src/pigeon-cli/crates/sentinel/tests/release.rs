//! `sentinel::release` against throwaway git repositories.

use std::fs;
use std::path::Path;
use std::process::Command;

use sentinel::release::{self, Config, Outcome};
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
