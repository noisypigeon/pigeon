//! Restacking a change onto its moved base; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use std::path::Path;
use std::process::Command;

use backplane::cobot::ci::{Relation, checkout, restack};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, path: &str, text: &str) {
    std::fs::write(dir.join(path), text).expect("writes");
}

/// A bare remote with `main`, and a work tree on it.
fn remote() -> (tempfile::TempDir, String, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tmp");
    let bare = tmp.path().join("remote.git");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("dir");
    git(
        tmp.path(),
        &["init", "-q", "--bare", bare.to_str().expect("utf-8")],
    );
    git(&work, &["init", "-q"]);
    write(&work, "a.txt", "one\ntwo\nthree\nfour\nfive\nsix\n");
    write(&work, "b.txt", "b\n");
    write(&work, "adr.md", "# ADR\n\nbody\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "base"]);
    let url = bare.to_str().expect("utf-8").to_string();
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/main"]);
    (tmp, url, work)
}

/// Commits `files` on a branch off `main`, then `moved` on `main`; the branch's head.
fn diverge(work: &Path, url: &str, files: &[(&str, &str)], moved: &[(&str, &str)]) -> String {
    git(work, &["checkout", "-q", "-B", "change/1-x", "main"]);
    for (p, t) in files {
        write(work, p, t);
    }
    git(work, &["commit", "-qam", "feat: x\n\nChange-Scope: a.txt"]);
    let head = git(work, &["rev-parse", "HEAD"]);
    git(work, &["push", "-q", url, "HEAD:refs/heads/change/1-x"]);
    git(work, &["checkout", "-q", "main"]);
    for (p, t) in moved {
        write(work, p, t);
    }
    if !moved.is_empty() {
        git(work, &["commit", "-qam", "other"]);
        git(work, &["push", "-q", url, "HEAD:refs/heads/main"]);
    }
    head
}

fn run(url: &str, head: &str) -> (tempfile::TempDir, backplane::cobot::ci::Restack) {
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), url, "", "main", "change/1-x", head).expect("checks out");
    let r = restack(job.path(), url, "", "main", "change/1-x", head, &["*.md"]).expect("restacks");
    (job, r)
}

#[test]
fn a_change_on_the_current_base_is_left_alone() {
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[],
    );
    let (_j, r) = run(&url, &head);
    assert_eq!(r.relation, Relation::Current);
    assert_eq!(r.to, None);
}

#[test]
fn a_disjoint_change_is_replayed_with_the_same_patch_and_its_message() {
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[("b.txt", "B\n")],
    );
    let (j, r) = run(&url, &head);
    assert_eq!(r.relation, Relation::Disjoint);
    assert!(r.same_patch);
    let to = r.to.expect("pushed");
    git(
        &work,
        &["fetch", "-q", &url, "+refs/heads/*:refs/remotes/r/*"],
    );
    assert_eq!(git(&work, &["rev-parse", "r/change/1-x"]), to);
    assert_eq!(
        git(&work, &["rev-parse", &format!("{to}^")]),
        git(&work, &["rev-parse", "r/main"])
    );
    assert!(git(j.path(), &["log", "-1", "--format=%B", &to]).contains("Change-Scope: a.txt"));
}

#[test]
fn a_clean_merge_of_the_same_file_is_replayed() {
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[("a.txt", "one\ntwo\nthree\nfour\nfive\nSIX\n")],
    );
    let (j, r) = run(&url, &head);
    assert_eq!(r.relation, Relation::Clean);
    let to = r.to.expect("pushed");
    assert_eq!(
        git(j.path(), &["show", &format!("{to}:a.txt")]),
        "ONE\ntwo\nthree\nfour\nfive\nSIX"
    );
}

#[test]
fn a_conflict_is_reported_and_nothing_is_pushed() {
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[("a.txt", "uno\ntwo\nthree\nfour\nfive\nsix\n")],
    );
    let (_j, r) = run(&url, &head);
    assert_eq!(r.relation, Relation::Conflicting);
    assert_eq!(r.conflicts, ["a.txt"]);
    assert_eq!(r.to, None);
    assert_eq!(r.context.len(), 1);
    assert_eq!(r.context[0].commits, ["other"]);
    assert!(
        r.context[0].base_diff.contains("+uno"),
        "{}",
        r.context[0].base_diff
    );
    assert!(r.context[0].change_diff.contains("+ONE"));
    assert!(r.messages.contains("other"));
    git(
        &work,
        &["fetch", "-q", &url, "+refs/heads/*:refs/remotes/r/*"],
    );
    assert_eq!(git(&work, &["rev-parse", "r/change/1-x"]), head);
}

#[test]
fn two_amendments_to_one_adr_merge_by_union() {
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("adr.md", "# ADR\n\nbody\n\n### B\n")],
        &[("adr.md", "# ADR\n\nbody\n\n### A\n")],
    );
    let (j, r) = run(&url, &head);
    assert_eq!(r.relation, Relation::Clean);
    let adr = git(
        j.path(),
        &["show", &format!("{}:adr.md", r.to.expect("pushed"))],
    );
    assert!(adr.contains("### A") && adr.contains("### B"), "{adr}");
}
