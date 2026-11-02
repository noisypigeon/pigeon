//! Which units release, against a small path-only cargo workspace.

use std::fs;
use std::path::Path;
use std::process::Command;

use cobot::release::{self, Config, Outcome};
use tempfile::TempDir;

const CONFIG: &str = r"description: test.
repository: https://x
units:
  a:
    manifest: ws/liba/Cargo.toml
    features: a
    paths: [ws/liba/, ws/app/src/commands/a.rs, ws/app/src/main.rs, ws/app/Cargo.toml]
    exclude: [tests/, '*.md']
    cargo: { workspace: ws, package: app }
  b:
    manifest: ws/libb/Cargo.toml
    features: b
    paths: [ws/libb/, ws/app/src/commands/b.rs, ws/app/src/main.rs, ws/app/Cargo.toml]
    exclude: [tests/, '*.md']
    cargo: { workspace: ws, package: app }
settings:
  - { file: ws/Cargo.toml, tables: '^\[profile\.' }
tiers:
  - { file: CHANGELOG.md, paths: [''] }
  - { file: ws/liba/CHANGELOG.md, unit: a }
  - { file: ws/libb/CHANGELOG.md, unit: b }
";

const WORKSPACE: &str = "[workspace]\nresolver = \"3\"\nmembers = [\"app\", \"liba\", \"libb\", \"extra\"]\n\n[workspace.dependencies]\nextra = { path = \"extra\" }\n";

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z")
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

fn lock(root: &Path) {
    let out = Command::new("cargo")
        .args(["generate-lockfile", "--offline", "--manifest-path"])
        .arg(root.join("ws/Cargo.toml"))
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit(root: &Path, subject: &str) {
    lock(root);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", subject]);
}

fn package(name: &str, extra: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{extra}")
}

/// A workspace where `app` builds `liba` for feature `a` and `libb` for
/// feature `b`, and only `liba` uses `extra`.
fn repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    write(root, "ws/Cargo.toml", WORKSPACE);
    write(
        root,
        "ws/app/Cargo.toml",
        &package(
            "app",
            "[features]\na = [\"dep:liba\"]\nb = [\"dep:libb\"]\n\n[dependencies]\nliba = { path = \"../liba\", optional = true }\nlibb = { path = \"../libb\", optional = true }\n",
        ),
    );
    write(root, "ws/app/src/main.rs", "fn main() {}\n");
    write(root, "ws/app/src/commands/a.rs", "\n");
    write(root, "ws/app/src/commands/b.rs", "\n");
    write(
        root,
        "ws/liba/Cargo.toml",
        &package("liba", "[dependencies]\nextra.workspace = true\n"),
    );
    write(root, "ws/liba/src/lib.rs", "\n");
    write(root, "ws/libb/Cargo.toml", &package("libb", ""));
    write(root, "ws/libb/src/lib.rs", "\n");
    write(
        root,
        "ws/extra/Cargo.toml",
        &package("extra", "[features]\nstd = []\n"),
    );
    write(root, "ws/extra/src/lib.rs", "\n");
    write(root, "releases.yaml", CONFIG);
    commit(root, "build: workspace (#1)");
    dir
}

fn run(root: &Path) -> Outcome {
    let config = Config::load(&root.join("releases.yaml")).expect("config");
    release::run(root, &config, None, "HEAD", None, false).expect("release")
}

/// Commits the bot's output and tags each release, as the workflow does.
fn land(root: &Path, outcome: &Outcome) {
    git(root, &["add", "-A"]);
    git(
        root,
        &["commit", "-q", "--allow-empty", "-m", "chore(release): x"],
    );
    for r in &outcome.released {
        git(root, &["tag", &r.tag]);
    }
}

fn released(o: &Outcome) -> Vec<String> {
    o.released.iter().map(|r| r.unit.clone()).collect()
}

/// A repository whose units have both had their first release.
fn released_repo() -> TempDir {
    let dir = repo();
    let first = run(dir.path());
    assert_eq!(released(&first), ["a", "b"]);
    land(dir.path(), &first);
    dir
}

fn change(root: &Path, rel: &str, content: &str, subject: &str) -> Outcome {
    write(root, rel, content);
    commit(root, subject);
    run(root)
}

#[test]
fn a_units_own_command_file_releases_it_alone() {
    let dir = released_repo();
    let o = change(
        dir.path(),
        "ws/app/src/commands/a.rs",
        "// a\n",
        "feat: a (#2)",
    );
    assert_eq!(released(&o), ["a"]);
}

#[test]
fn shared_code_releases_every_unit() {
    let dir = released_repo();
    let o = change(
        dir.path(),
        "ws/app/src/main.rs",
        "fn main() { }\n",
        "fix: main (#2)",
    );
    assert_eq!(released(&o), ["a", "b"]);
}

#[test]
fn tests_and_docs_release_nothing() {
    let dir = released_repo();
    write(dir.path(), "ws/liba/tests/t.rs", "\n");
    let o = change(dir.path(), "ws/liba/README.md", "# a\n", "docs: a (#2)");
    assert_eq!(released(&o), Vec::<String>::new());
}

#[test]
fn a_dependency_crate_releases_only_the_units_that_build_it() {
    let dir = released_repo();
    let o = change(
        dir.path(),
        "ws/extra/src/lib.rs",
        "// x\n",
        "fix: extra (#2)",
    );
    assert_eq!(released(&o), ["a"]);
}

#[test]
fn a_feature_only_dependency_change_releases_its_units() {
    let dir = released_repo();
    let o = change(
        dir.path(),
        "ws/Cargo.toml",
        &WORKSPACE.replace(
            "extra = { path = \"extra\" }",
            "extra = { path = \"extra\", features = [\"std\"] }",
        ),
        "build: extra std (#2)",
    );
    assert_eq!(released(&o), ["a"]);
    let log = fs::read_to_string(dir.path().join("ws/liba/CHANGELOG.md")).expect("read");
    assert!(
        log.contains("- build: extra std (dependencies) ([#2](https://x/pull/2))"),
        "{log}"
    );
}

#[test]
fn build_settings_release_every_unit_and_other_lines_nothing() {
    let dir = released_repo();
    let o = change(
        dir.path(),
        "ws/Cargo.toml",
        &format!("{WORKSPACE}\n[workspace.package]\nlicense = \"MIT\"\n"),
        "chore: license (#2)",
    );
    assert_eq!(released(&o), Vec::<String>::new());
    land(dir.path(), &o);
    let o = change(
        dir.path(),
        "ws/Cargo.toml",
        &format!(
            "{WORKSPACE}\n[workspace.package]\nlicense = \"MIT\"\n\n[profile.release]\nlto = true\n"
        ),
        "perf: lto (#3)",
    );
    assert_eq!(released(&o), ["a", "b"]);
}

#[test]
fn a_first_release_lists_only_its_own_crate() {
    let dir = repo();
    let root = dir.path();
    write(root, "ws/libb/src/lib.rs", "// b\n");
    commit(root, "feat: b only (#2)");
    let first = run(root);
    let a = fs::read_to_string(root.join("ws/liba/CHANGELOG.md")).expect("read");
    let b = fs::read_to_string(root.join("ws/libb/CHANGELOG.md")).expect("read");
    assert!(!a.contains("#2"), "{a}");
    assert!(b.contains("#2") && b.contains("#1"), "{b}");
    assert_eq!(released(&first), ["a", "b"]);
}
