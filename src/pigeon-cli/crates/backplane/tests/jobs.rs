//! A job Container's work against a real checkout: this repository, cloned bare, with a change
//! branch on top; see `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use backplane::changes::{BOUND, MERGE};
use backplane::control::{JobSpec, Outcome};
use backplane::jobs::{self, Board};
use tempfile::TempDir;

mod common;

/// Runs `spec` with the built `pigeon-sentinel` as the bound job's judge.
fn run_job(spec: &JobSpec, dir: &Path) -> backplane::control::Report {
    jobs::run_judged(spec, dir, common::sentinel())
}

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
    remote_removing(tmp, paths, &[])
}

/// As [`remote`], the change also deleting `removed`.
fn remote_removing(tmp: &Path, paths: &[&str], removed: &[&str]) -> (PathBuf, String) {
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
    for p in removed {
        git(&work, &["rm", "-q", p]);
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
    let r = run_job(
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
            "test pigeon-backplane: cobot_catalog cobot_manifest cobot_apex cobot_git cobot_credentials"
        ],
        "policy.check is the bound job's own"
    );
    assert!(r.log.contains("2 file(s) changed"), "{}", r.log);
}

#[test]
fn the_bound_job_judges_with_mains_policies_not_the_changes_own() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote_removing(
        tmp.path(),
        &["mise.toml"],
        &[".config/policies/change.yaml"],
    );
    let r = run_job(
        &spec(&bare, &sha, BOUND, &[".config/policies/**"], &[]),
        &tmp.path().join("job"),
    );
    assert_eq!(r.outcome, Outcome::Failed, "{}", r.log);
    assert!(
        r.violations
            .iter()
            .any(|v| v.contains("outside `scope`: mise.toml")),
        "main's change policy still bounds it: {:?}",
        r.violations
    );
}

#[test]
fn the_bound_job_refuses_a_file_outside_the_approved_scope_and_a_stale_head() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md", "mise.toml"]);
    let r = run_job(
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
    let r = run_job(
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
    let r = run_job(
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
    let r = run_job(
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
    s.env.insert(
        backplane::control::COMMIT_AUTHOR.into(),
        "Owner <1+owner@users.noreply.github.com>".into(),
    );
    let r = run_job(&s, &tmp.path().join("m"));
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
    assert!(message.contains("Change-Approved-By: owner\n"), "{message}");
    assert!(
        !message.contains("@example.com"),
        "no approver's email: {message}"
    );
    assert_eq!(
        git(&bare, &["log", "-1", "--format=%an <%ae>", &commit]),
        "Owner <1+owner@users.noreply.github.com>"
    );
    assert_eq!(
        git(&bare, &["log", "-1", "--format=%cn <%ce>", &commit]),
        "pigeon <pigeon@users.noreply.pigeon.dev>",
        "the control plane commits it"
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
fn a_merge_with_no_author_is_pigeons_and_a_malformed_author_is_refused() {
    let tmp = TempDir::new().expect("tempdir");
    let (bare, sha) = remote(tmp.path(), &["CLAUDE.md"]);
    let mut s = spec(&bare, &sha, MERGE, &["CLAUDE.md"], &[]);
    s.approver = Some("grant:owner@example.com".into());
    s.env.insert(
        backplane::control::COMMIT_AUTHOR.into(),
        "no-angle-brackets@example.com".into(),
    );
    let r = run_job(&s, &tmp.path().join("bad"));
    assert_eq!(r.outcome, Outcome::Failed, "{}", r.log);
    assert!(r.violations.join(" ").contains("is not `Name <email>`"));

    s.env.clear();
    let r = run_job(&s, &tmp.path().join("m"));
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    let commit = r.merged.expect("merged");
    assert_eq!(
        git(&bare, &["log", "-1", "--format=%an <%ae>", &commit]),
        "pigeon <pigeon@users.noreply.pigeon.dev>"
    );
    let message = git(&bare, &["log", "-1", "--format=%B", &commit]);
    assert!(
        message.contains("Change-Approved-By: grant:owner\n"),
        "{message}"
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
    let r = run_job(
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
        run_job(&s, &tmp.path().join(dir))
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
    let first = run_job(&s, &tmp.path().join("m1"));
    assert_eq!(first.outcome, Outcome::Passed, "{}", first.log);
    let commit = first.merged.expect("merged");
    let again = run_job(&s, &tmp.path().join("m2"));
    assert_eq!(again.outcome, Outcome::Passed, "{}", again.log);
    assert_eq!(again.merged.as_deref(), Some(commit.as_str()));
    assert!(again.log.contains("already on main"), "{}", again.log);
    assert_eq!(
        git(&bare, &["rev-list", "--count", "refs/heads/main"]),
        "2",
        "merged once"
    );
}

#[test]
fn a_platform_apply_checks_out_where_its_plan_did() {
    let sha = "0123456789abcdef0123";
    let mut s = spec(Path::new("/x"), sha, backplane::control::PLATFORM, &[], &[]);
    for (id, dir) in [
        ("platform-14-plan", "platform-14-plan"),
        ("platform-14-apply", "platform-14-plan"),
    ] {
        s.id = id.into();
        assert_eq!(jobs::dir_name(&s), dir);
    }
    let mut other = spec(Path::new("/x"), sha, BOUND, &[], &[]);
    other.id = "7-abc-apply".into();
    assert_eq!(
        jobs::dir_name(&other),
        "7-abc-apply",
        "only platform jobs share a directory"
    );
}

#[test]
fn a_mirror_job_moves_main_and_tags_and_never_shows_its_token() {
    let tmp = TempDir::new().expect("tempdir");
    let from = tmp.path().join("artifacts.git");
    let to = tmp.path().join("github.git");
    let work = tmp.path().join("work");
    git(
        tmp.path(),
        &["init", "-q", "--bare", from.to_str().expect("utf-8")],
    );
    git(
        tmp.path(),
        &["init", "-q", "--bare", to.to_str().expect("utf-8")],
    );
    git(
        tmp.path(),
        &["init", "-q", "-b", "main", work.to_str().expect("utf-8")],
    );
    std::fs::write(work.join("a.txt"), "a\n").expect("write");
    git(&work, &["add", "a.txt"]);
    git(&work, &["commit", "-q", "-m", "a"]);
    git(&work, &["tag", "v1"]);
    let from_path = from.to_str().expect("utf-8");
    git(&work, &["push", "-q", from_path, "main", "v1"]);
    git(&work, &["push", "-q", from_path, "main:change/2-x"]);
    let sha = git(&work, &["rev-parse", "HEAD"]);
    let to_url = format!("file://{}", to.display());
    let s = backplane::control::mirror_spec(
        &format!("file://{}", from.display()),
        "a-token-never-shown",
        &sha,
        &to_url,
        "ghs-never-shown",
    );
    let r = run_job(&s, &tmp.path().join("job"));
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    assert_eq!(git(&to, &["rev-parse", "refs/heads/main"]), sha);
    assert_eq!(git(&to, &["rev-parse", "refs/tags/v1^{commit}"]), sha);
    assert_eq!(
        git(&to, &["for-each-ref", "--format=%(refname)", "refs/heads"]),
        "refs/heads/main",
        "change/* never reaches the mirror"
    );
    assert!(!r.log.contains("ghs-never-shown"));

    let gone = backplane::control::mirror_spec(
        &format!("file://{}", from.display()),
        "a-token-never-shown",
        &sha,
        &format!("file://{}", tmp.path().join("gone.git").display()),
        "ghs-never-shown",
    );
    let r = run_job(&gone, &tmp.path().join("job"));
    assert_eq!(r.outcome, Outcome::Failed);
    let shown = serde_json::to_string(&r).expect("json");
    assert!(!shown.contains("ghs-never-shown"), "{shown}");
}

#[test]
fn a_bounded_change_head_reaches_its_branch_on_the_mirror_and_leaves_it_once_done() {
    let tmp = TempDir::new().expect("tempdir");
    let (from, sha) = remote(tmp.path(), &["docs/x.md"]);
    let to = tmp.path().join("github.git");
    git(
        tmp.path(),
        &["init", "-q", "--bare", to.to_str().expect("utf-8")],
    );
    let to_url = format!("file://{}", to.display());
    let mut s = spec(&from, &sha, backplane::control::MIRROR, &[], &[]);
    s.env = std::collections::BTreeMap::from([
        (backplane::control::MIRROR_TO.into(), to_url.clone()),
        (
            backplane::control::MIRROR_TOKEN.into(),
            "ghs-never-shown".into(),
        ),
    ]);
    let r = run_job(&s, &tmp.path().join("job"));
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    assert_eq!(git(&to, &["rev-parse", "refs/heads/change/1-x"]), sha);
    assert_eq!(
        git(
            &to,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads",
                "refs/tags"
            ]
        ),
        "refs/heads/change/1-x",
        "only the change's branch: main and tags go by the main mirror"
    );
    assert!(
        !serde_json::to_string(&r)
            .expect("json")
            .contains("ghs-never-shown")
    );

    let mut change = backplane::changes::Change::propose(1, "x", "docs: x", "p", vec![], "a");
    change.head = Some(sha);
    let u = backplane::control::unmirror_spec(&change, &to_url, "ghs-never-shown");
    let r = run_job(&u, &tmp.path().join("job"));
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    assert_eq!(
        git(&to, &["for-each-ref", "--format=%(refname)", "refs/heads"]),
        ""
    );
    let r = run_job(&u, &tmp.path().join("job"));
    assert_eq!(
        r.outcome,
        Outcome::Passed,
        "a branch already gone: {}",
        r.log
    );
}

fn sentinel_spec() -> JobSpec {
    serde_json::from_value(serde_json::json!({
        "id": "7-x", "remote": "file:///r", "token": "t", "base": "main",
        "branch": "change/7-x", "sha": "a", "number": 7, "title": "fix: x",
        "plan": "a plan", "scope": ["a/**", "b"], "job": "bound",
    }))
    .expect("a spec")
}

/// A stand-in `pigeon-sentinel` in `dir` running `body`.
fn stand_in(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let p = dir.join("pigeon-sentinel");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    p
}

#[test]
fn the_sentinel_binary_is_asked_with_the_change_as_its_pull_request() {
    let tmp = TempDir::new().expect("tempdir");
    let bin = stand_in(
        tmp.path(),
        r#"printf '{"ran":1,"skipped":0,"violations":["%s|%s|%s|%s"]}' "$PR_TITLE" "$PR_NUMBER" "$(printf %s "$PR_SCOPE" | tr '\n' ,)" "$*"; exit 1"#,
    );
    let got = jobs::judge_with(
        &bin,
        &sentinel_spec(),
        Path::new("/head"),
        &["a/1".into()],
        Path::new("/main-policies"),
    )
    .expect("judged");
    assert_eq!(
        got,
        ["fix: x|7|a/**,b|check --json --pull-request --root /head --policies /main-policies"]
    );
}

#[test]
fn a_sentinel_binary_that_cant_judge_is_an_error_not_a_pass() {
    let tmp = TempDir::new().expect("tempdir");
    let bad = stand_in(tmp.path(), "echo 'p.yaml: bad' >&2; exit 2");
    let why = jobs::judge_with(
        &bad,
        &sentinel_spec(),
        Path::new("/h"),
        &[],
        Path::new("/p"),
    )
    .expect_err("refused");
    assert!(why.contains("p.yaml: bad"), "{why}");
    let garbled = stand_in(tmp.path(), "echo not-json; exit 0");
    assert!(
        jobs::judge_with(
            &garbled,
            &sentinel_spec(),
            Path::new("/h"),
            &[],
            Path::new("/p")
        )
        .is_err()
    );
}

/// An eviction job on the remote's `main`, judging `candidates`.
fn eviction_job(tmp: &Path, candidates: &serde_json::Value) -> JobSpec {
    let (bare, _) = remote(tmp, &["x.txt"]);
    let main = git(&bare, &["rev-parse", "main"]);
    backplane::control::eviction_spec(
        &format!("file://{}", bare.display()),
        "a-token-never-shown",
        &main,
        candidates,
        1,
    )
}

#[test]
fn the_eviction_job_asks_mains_sentinel_about_each_candidate() {
    let tmp = TempDir::new().expect("tempdir");
    let candidates = serde_json::json!([
        { "number": 79, "fields": { "number": "79", "state": "pushed" } },
        { "number": 80, "fields": { "number": "80", "state": "approved" } },
    ]);
    let spec = eviction_job(tmp.path(), &candidates);
    assert_eq!(
        backplane::control::class_of(&spec.job),
        backplane::control::Class::Runner
    );
    let bin = stand_in(
        tmp.path(),
        r#"case "$*" in
  *'"number":"79"'*) printf '{"granted":"eviction-session-gone","results":[]}'; exit 0;;
  *) printf '{"granted":null,"results":[{"policy":"eviction-session-gone","violations":["approved"]}]}'; exit 1;;
esac"#,
    );
    let r = jobs::run_judged(&spec, &tmp.path().join("job"), &bin);
    assert_eq!(r.outcome, Outcome::Passed, "{}", r.log);
    let granted: Vec<(u64, serde_json::Value)> = r
        .evictions
        .iter()
        .map(|v| {
            (
                v["number"].as_u64().unwrap_or_default(),
                v["granted"].clone(),
            )
        })
        .collect();
    assert_eq!(
        granted,
        [
            (79, serde_json::json!("eviction-session-gone")),
            (80, serde_json::Value::Null)
        ]
    );
    let judged = backplane::control::Judged {
        verdicts: r.evictions,
        passed: true,
        ..Default::default()
    };
    assert_eq!(judged.granted(), [79]);
}

#[test]
fn an_eviction_job_whose_judge_cant_answer_fails_and_grants_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let candidates = serde_json::json!([{ "number": 79, "fields": {} }]);
    let spec = eviction_job(tmp.path(), &candidates);
    let bad = stand_in(tmp.path(), "echo 'eviction-x.yaml: bad' >&2; exit 2");
    let r = jobs::run_judged(&spec, &tmp.path().join("job"), &bad);
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.evictions.len(), 0);
    assert!(r.log.contains("eviction-x.yaml: bad"), "{}", r.log);
    let mut garbled = spec.clone();
    garbled
        .env
        .insert(backplane::control::CANDIDATES.into(), "not json".into());
    let r = jobs::run_judged(&garbled, &tmp.path().join("job2"), &bad);
    assert_eq!(r.outcome, Outcome::Failed);
}
