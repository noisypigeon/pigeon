//! A job Container's work against a real checkout: this repository, cloned bare, with a change
//! branch on top; see `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use backplane::changes::{BOUND, MERGE};
use backplane::control::{JobSpec, Outcome};
use backplane::jobs::{self, Board};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare remote holding this repository's HEAD as `main`, and `change/1-x` one commit on top
/// touching `paths`; the remote and the branch's head.
fn remote(tmp: &Path, paths: &[&str]) -> (PathBuf, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let bare = tmp.join("remote.git");
    let work = tmp.join("work");
    let bare_path = bare.to_str().expect("utf-8");
    git(tmp, &["init", "-q", "--bare", bare_path]);
    git(
        tmp,
        &["init", "-q", "-b", "main", work.to_str().expect("utf-8")],
    );
    // A snapshot of HEAD, so a shallow checkout works too.
    let archived = Command::new("sh")
        .arg("-c")
        .arg("git -C \"$0\" archive HEAD | tar -x -C \"$1\"")
        .arg(&root)
        .arg(&work)
        .status()
        .expect("archive");
    assert!(archived.success());
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "main"]);
    git(&work, &["remote", "add", "origin", bare_path]);
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["checkout", "-q", "-b", "change/1-x"]);
    for p in paths {
        let f = work.join(p);
        std::fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
        std::fs::write(&f, "changed\n").expect("write");
        git(&work, &["add", p]);
    }
    git(&work, &["commit", "-q", "-m", "docs: x"]);
    git(&work, &["push", "-q", "origin", "change/1-x"]);
    let sha = git(&work, &["rev-parse", "HEAD"]);
    (bare, sha)
}

fn spec(remote: &Path, sha: &str, job: &str, scope: &[&str], argv: &[&str]) -> JobSpec {
    JobSpec {
        id: format!("1-{}-{job}", &sha[..12]),
        remote: format!("file://{}", remote.display()),
        token: "a-token-never-shown".into(),
        base: "main".into(),
        branch: "change/1-x".into(),
        sha: sha.into(),
        number: 1,
        title: "docs: x".into(),
        plan: "Per .claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md.".into(),
        scope: scope.iter().map(|s| (*s).to_string()).collect(),
        job: job.into(),
        argv: argv.iter().map(|s| (*s).to_string()).collect(),
        approver: None,
        since: None,
        env: std::collections::BTreeMap::new(),
    }
}

#[test]
fn the_bound_job_passes_a_change_inside_its_scope_and_names_its_checks() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md", "catalog/cloudflare/kinds/x.md"]);
    let r = jobs::run(
        &spec(&bare, &sha, BOUND, &["CLAUDE.md", "catalog/**"], &[]),
        &tmp.path().join("job"),
    );
    assert_eq!(r.violations, Vec::<String>::new(), "{}", r.log);
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    let names: Vec<&str> = r.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "infra.validate",
            "test cobot (catalog, manifest, apex, git, credentials)"
        ],
        "policy.check is the bound job's own"
    );
    assert!(r.log.contains("2 file(s) changed"), "{}", r.log);
}

#[test]
fn the_bound_job_refuses_a_file_outside_the_approved_scope_and_a_stale_head() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md", "mise.toml"]);
    let r = jobs::run(
        &spec(&bare, &sha, BOUND, &["CLAUDE.md"], &[]),
        &tmp.path().join("a"),
    );
    assert_eq!(r.outcome, Outcome::Failed);
    assert!(
        r.violations
            .iter()
            .any(|v| v.contains("outside `scope`: mise.toml")),
        "{:?}",
        r.violations
    );
    assert!(r.checks.is_empty(), "a refused head runs no checks");
    let stale = "0123456789abcdef0123456789abcdef01234567";
    let r = jobs::run(
        &spec(&bare, stale, BOUND, &["**"], &[]),
        &tmp.path().join("b"),
    );
    assert_eq!(r.outcome, Outcome::Failed);
    assert!(
        r.violations[0].contains("not 0123456789ab"),
        "{:?}",
        r.violations
    );
}

#[test]
fn a_check_runs_in_the_workspace_and_its_token_is_cut_from_the_log() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md"]);
    let r = jobs::run(
        &spec(
            &bare,
            &sha,
            "fmt",
            &["**"],
            &["sh", "-c", "pwd; echo a-token-never-shown; exit 3"],
        ),
        &tmp.path().join("c"),
    );
    assert_eq!(r.outcome, Outcome::Failed);
    assert!(r.log.contains("src/pigeon-cli"), "{}", r.log);
    assert!(!r.log.contains("a-token-never-shown"), "{}", r.log);
    let r = jobs::run(
        &spec(&bare, &sha, "ok", &["**"], &["true"]),
        &tmp.path().join("d"),
    );
    assert_eq!(r.outcome, Outcome::Passed);
}

#[test]
fn the_board_runs_a_job_once_and_reports_it() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md"]);
    let board = Arc::new(Board::new(Some(tmp.path().join("work"))));
    let s = spec(&bare, &sha, "ok", &["**"], &["true"]);
    let body = serde_json::to_vec(&s).expect("json");
    let (status, v) = jobs::submit(&board, &body);
    assert_eq!(status, 202);
    assert_eq!(v["outcome"], "running");
    for _ in 0..200 {
        if board
            .get(&s.id)
            .is_some_and(|r| r.outcome != Outcome::Running)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(jobs::status(&board, &s.id).1["outcome"], "passed");
    assert_eq!(jobs::status(&board, "nope").0, 404);
    let mut bad = s.clone();
    bad.id = "../x".into();
    assert_eq!(
        jobs::submit(&board, &serde_json::to_vec(&bad).expect("json")).0,
        400
    );
}

#[test]
fn the_merge_job_squashes_the_checked_tree_onto_main_and_deletes_the_branch() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md"]);
    let mut s = spec(&bare, &sha, MERGE, &["CLAUDE.md"], &[]);
    s.approver = Some("owner@example.com".into());
    let r = jobs::run(&s, &tmp.path().join("m"));
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    let commit = r.merged.expect("merged");
    assert_eq!(git(&bare, &["rev-parse", "refs/heads/main"]), commit);
    assert_eq!(
        git(&bare, &["rev-parse", &format!("{commit}^{{tree}}")]),
        git(&bare, &["rev-parse", &format!("{sha}^{{tree}}")]),
        "main holds exactly the checked tree"
    );
    let message = git(&bare, &["log", "-1", "--format=%B", &commit]);
    assert!(message.starts_with("docs: x (!1)\n"), "{message}");
    assert!(
        message.contains("Change-Approved-By: owner@example.com"),
        "{message}"
    );
    assert!(
        message.contains(&format!("Change-Head: {sha}")),
        "{message}"
    );
    assert_eq!(
        git(&bare, &["rev-list", "--count", "refs/heads/main"]),
        "2",
        "one commit on main"
    );
    assert_eq!(
        git(&bare, &["branch", "--list", "change/1-x"]),
        "",
        "the branch is gone"
    );
}

#[test]
fn the_merge_job_refuses_when_main_has_moved_past_the_branch() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md"]);
    let work = tmp.path().join("work");
    git(&work, &["checkout", "-q", "main"]);
    std::fs::write(work.join("README.md"), "moved\n").expect("write");
    git(&work, &["add", "README.md"]);
    git(&work, &["commit", "-q", "-m", "docs: moved"]);
    git(&work, &["push", "-q", "origin", "main"]);
    let main = git(&bare, &["rev-parse", "refs/heads/main"]);
    let r = jobs::run(
        &spec(&bare, &sha, MERGE, &["CLAUDE.md"], &[]),
        &tmp.path().join("m"),
    );
    assert_eq!(r.outcome, Outcome::Failed);
    assert!(
        r.violations[0].contains("rebase onto main"),
        "{:?}",
        r.violations
    );
    assert_eq!(
        git(&bare, &["rev-parse", "refs/heads/main"]),
        main,
        "main is untouched"
    );
    assert_ne!(
        git(&bare, &["branch", "--list", "change/1-x"]),
        "",
        "the branch stays"
    );
}

#[test]
fn a_deploy_publishes_only_the_sites_whose_inputs_changed() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, _) = remote(tmp.path(), &["CLAUDE.md"]);
    let work = tmp.path().join("work");
    git(&work, &["checkout", "-q", "main"]);
    let before = git(&work, &["rev-parse", "HEAD"]);
    std::fs::write(work.join("CLAUDE.md"), "docs only\n").expect("write");
    git(&work, &["commit", "-q", "-am", "docs: only"]);
    git(&work, &["push", "-q", "origin", "main"]);
    let docs_only = git(&work, &["rev-parse", "HEAD"]);
    let deploy = |sha: &str, since: Option<&str>, dir: &str| {
        let s = backplane::control::deploy_spec(
            &format!("file://{}", bare.display()),
            "a-token-never-shown",
            sha,
            since,
            std::collections::BTreeMap::new(),
        );
        jobs::run(&s, &tmp.path().join(dir))
    };
    let r = deploy(&docs_only, Some(&before), "a");
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    assert!(r.log.contains("no site to publish"), "{}", r.log);
    assert!(r.log.contains("no Worker to deploy"), "{}", r.log);
    let mut config = std::fs::read_to_string(work.join("config.yaml")).expect("read");
    config.push_str("# a change the architecture site draws\n");
    std::fs::write(work.join("config.yaml"), config).expect("write");
    git(&work, &["commit", "-q", "-am", "chore: config"]);
    git(&work, &["push", "-q", "origin", "main"]);
    let changed = git(&work, &["rev-parse", "HEAD"]);
    let r = deploy(&changed, Some(&docs_only), "b");
    assert_eq!(r.outcome, Outcome::Failed, "{}", r.log);
    assert!(
        r.violations
            .iter()
            .any(|v| v.contains("CLOUDFLARE_API_TOKEN")),
        "sites/docs is published, which needs the deploy token: {:?}",
        r.violations
    );
    let r = deploy(&docs_only, Some(&before), "c");
    assert_eq!(
        r.outcome,
        Outcome::Failed,
        "a SHA that isn't main's head is refused"
    );
}

#[test]
fn a_merge_whose_report_was_lost_is_found_on_main_when_run_again() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md"]);
    let mut s = spec(&bare, &sha, MERGE, &["CLAUDE.md"], &[]);
    s.approver = Some("owner@example.com".into());
    let first = jobs::run(&s, &tmp.path().join("m1"));
    assert_eq!(first.outcome, Outcome::Passed, "{}", first.log);
    let commit = first.merged.expect("merged");
    let again = jobs::run(&s, &tmp.path().join("m2"));
    assert_eq!(again.outcome, Outcome::Passed, "{}", again.log);
    assert_eq!(again.merged.as_deref(), Some(commit.as_str()));
    assert!(again.log.contains("already on main"), "{}", again.log);
    assert_eq!(
        git(&bare, &["rev-list", "--count", "refs/heads/main"]),
        "2",
        "merged once"
    );
}
