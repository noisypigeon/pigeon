//! Restacking a change onto its moved base; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use std::path::Path;
use std::process::Command;

use backplane::cobot::ci::{
    Relation, built_on_head, changed, checkout, fetch_branch, forecast, restack,
};

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
    let r = restack(
        job.path(),
        url,
        "",
        "main",
        "change/1-x",
        head,
        None,
        &["*.md"],
    )
    .expect("restacks");
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
fn a_change_built_on_the_one_that_added_a_file_keeps_its_edits_to_it() {
    // !375, built on !374, edited a line of the ADR !374 added; restacked from main's fork, the
    // ADR was an add/add, and the union driver kept main's line beside or over its own.
    let (_t, url, work) = remote();
    let adr = "# ADR\n\n* Governs: `a`\n\nbody\n\n## Amendments\n\n### one\n\n- 1\n";
    let parent = diverge(&work, &url, &[("adr.md", adr)], &[]);
    git(&work, &["checkout", "-q", "-B", "change/2-y", &parent]);
    let edited = adr.replace("`a`", "`a`, `b`") + "\n### two\n\n- 2\n";
    write(&work, "adr.md", &edited);
    git(
        &work,
        &["commit", "-qam", "feat: y\n\nChange-Scope: adr.md"],
    );
    let head = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/change/2-y"]);
    // change/1-x merges as a squash, then another amendment lands on main.
    git(&work, &["checkout", "-q", "main"]);
    write(&work, "adr.md", adr);
    git(&work, &["add", "adr.md"]);
    git(&work, &["commit", "-qm", "feat: x (!1)"]);
    write(&work, "adr.md", &format!("{adr}\n### three\n\n- 3\n"));
    git(&work, &["commit", "-qam", "docs: three (!3)"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/main"]);
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/2-y", &head).expect("checks out");
    let r = restack(
        job.path(),
        &url,
        "",
        "main",
        "change/2-y",
        &head,
        Some(&parent),
        &["*.md"],
    )
    .expect("restacks");
    let to = r.to.expect("pushed");
    let merged = git(job.path(), &["show", &format!("{to}:adr.md")]);
    let governs: Vec<&str> = merged.lines().filter(|l| l.contains("Governs")).collect();
    assert_eq!(governs, ["* Governs: `a`, `b`"], "{merged}");
    assert!(
        merged.contains("### two") && merged.contains("### three"),
        "{merged}"
    );
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

#[test]
fn an_amendment_restacked_under_another_keeps_its_patch() {
    // !371 and !372 each lost their approval to a union merge's shared blank line; see the
    // concurrent-sessions ADR's 2026-11-13 amendment on carries.
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("adr.md", "# ADR\n\nbody\n\n### B\n\n- b\n")],
        &[("adr.md", "# ADR\n\nbody\n\n### A\n\n- a\n")],
    );
    let (_j, r) = run(&url, &head);
    assert_eq!(r.relation, Relation::Clean);
    assert!(r.same_patch, "its own lines are unchanged");
}

#[test]
fn a_changed_line_is_a_changed_patch() {
    let (_t, url, work) = remote();
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[("a.txt", "one\ntwo\nthree\nfour\nfive\nSIX\n")],
    );
    let (j, r) = run(&url, &head);
    assert!(r.same_patch, "a distant line of main's leaves it");
    let base = git(j.path(), &["rev-parse", "HEAD"]);
    let own = backplane::cobot::ci::own_lines(j.path(), &base, &base).expect("diff");
    assert_eq!(own, Some(Vec::new()));
}

#[test]
fn a_change_whose_patch_already_landed_on_main_is_reported_and_left_alone() {
    let (_t, url, work) = remote();
    let edit = ("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n");
    // main took the same edit, as a squash of another change carrying it does.
    let head = diverge(&work, &url, &[edit], &[edit, ("b.txt", "B\n")]);
    let (_j, r) = run(&url, &head);
    assert!(r.landed);
    assert_eq!(r.relation, Relation::Clean);
    assert_eq!(r.to, None, "nothing pushed");
    git(
        &work,
        &["fetch", "-q", &url, "+refs/heads/*:refs/remotes/r/*"],
    );
    assert_eq!(
        git(&work, &["rev-parse", "r/change/1-x"]),
        head,
        "the branch keeps its head"
    );
}

#[test]
fn a_forecast_names_the_branches_a_head_would_conflict_with_and_pushes_nothing() {
    let (_t, url, work) = remote();
    // change/1-x edits a.txt's first line.
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[],
    );
    // change/2-y edits the same line differently; change/3-z another file.
    let other = |branch: &str, file: &str, text: &str| {
        git(&work, &["checkout", "-q", "-B", branch, "main"]);
        write(&work, file, text);
        git(&work, &["commit", "-qam", branch]);
        git(
            &work,
            &["push", "-q", &url, &format!("HEAD:refs/heads/{branch}")],
        );
    };
    other("change/2-y", "a.txt", "uno\ntwo\nthree\nfour\nfive\nsix\n");
    other("change/3-z", "b.txt", "bee\n");
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/1-x", &head).expect("checks out");
    let others = [
        (2, "change/2-y".to_string()),
        (3, "change/3-z".to_string()),
        (4, "change/4-gone".to_string()),
    ];
    let found = forecast(job.path(), &url, "", "main", &head, &others, &["*.md"]);
    assert_eq!(
        found,
        [(2, vec!["a.txt".to_string()])],
        "only the conflicting one; a missing branch is skipped"
    );
    git(
        &work,
        &["fetch", "-q", &url, "+refs/heads/*:refs/remotes/r/*"],
    );
    assert_eq!(
        git(&work, &["rev-parse", "r/change/1-x"]),
        head,
        "nothing pushed"
    );
}

#[test]
fn a_change_built_on_another_counts_its_own_files_and_restacks_with_its_own_message() {
    let (_t, url, work) = remote();
    // change/1-x edits a.txt; change/2-y is built on it and edits b.txt.
    let parent = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[],
    );
    git(&work, &["checkout", "-q", "-B", "change/2-y", &parent]);
    write(&work, "b.txt", "B\n");
    git(&work, &["commit", "-qam", "feat: y\n\nChange-Scope: b.txt"]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/change/2-y"]);

    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/2-y", &head).expect("checks out");
    assert_eq!(changed(job.path(), "main", &head).expect("diff").len(), 2);
    fetch_branch(job.path(), &url, "", "change/1-x").expect("fetches");
    assert_eq!(
        changed(job.path(), "change/1-x", &head).expect("diff"),
        ["b.txt"],
        "only its own files"
    );

    // change/1-x merges as a squash; change/2-y restacks past its last head.
    git(&work, &["checkout", "-q", "main"]);
    write(&work, "a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n");
    git(&work, &["commit", "-qam", "feat: x (!1)"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/main"]);
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/2-y", &head).expect("checks out");
    let r = restack(
        job.path(),
        &url,
        "",
        "main",
        "change/2-y",
        &head,
        Some(&parent),
        &["*.md"],
    )
    .expect("restacks");
    let to = r.to.expect("pushed");
    let message = git(job.path(), &["log", "-1", "--format=%B", &to]);
    assert!(message.starts_with("feat: y"), "{message}");
    git(
        &work,
        &["fetch", "-q", &url, "+refs/heads/*:refs/remotes/r/*"],
    );
    assert_eq!(
        git(&work, &["diff", "--name-only", "r/main", &to]),
        "b.txt",
        "on main, only its own change"
    );
}

#[test]
fn main_drift_under_an_older_branch_is_not_forecast() {
    let (_t, url, work) = remote();
    // change/2-y is cut from main, then main rewrites b.txt's line.
    git(&work, &["checkout", "-q", "-B", "change/2-y", "main"]);
    write(&work, "b.txt", "bee\n");
    git(&work, &["commit", "-qam", "change/2-y"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/change/2-y"]);
    git(&work, &["checkout", "-q", "main"]);
    write(&work, "b.txt", "BEE\n");
    git(&work, &["commit", "-qam", "main moves"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/main"]);
    // change/1-x, cut from the new main, edits only a.txt.
    let head = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[],
    );
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/1-x", &head).expect("checks out");
    let others = [(2, "change/2-y".to_string())];
    assert_eq!(
        forecast(job.path(), &url, "", "main", &head, &others, &["*.md"]),
        Vec::<(u32, Vec<String>)>::new(),
        "b.txt is main's conflict with !2, not !1's"
    );
}

#[test]
fn two_amendments_to_one_adr_are_not_forecast_to_conflict() {
    let (_t, url, work) = remote();
    git(&work, &["checkout", "-q", "-B", "change/2-y", "main"]);
    write(&work, "adr.md", "# ADR\n\nbody\n\n## y\n");
    git(&work, &["commit", "-qam", "change/2-y"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/change/2-y"]);
    let head = diverge(&work, &url, &[("adr.md", "# ADR\n\nbody\n\n## x\n")], &[]);
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/1-x", &head).expect("checks out");
    let others = [(2, "change/2-y".to_string())];
    assert_eq!(
        forecast(job.path(), &url, "", "main", &head, &others, &["*.md"]),
        Vec::<(u32, Vec<String>)>::new(),
        "appends merge by union"
    );
    assert_eq!(
        forecast(job.path(), &url, "", "main", &head, &others, &[]),
        [(2, vec!["adr.md".to_string()])],
        "and conflict without it"
    );
}

#[test]
fn a_change_built_on_one_restacked_before_merging_keeps_its_own_message() {
    // !386 was built on !384, which restacked at its turn before merging; !386's restack then
    // took !384's message. See the yield ADR's 2026-11-14 amendment on `after`.
    let (_t, url, work) = remote();
    let parent = diverge(
        &work,
        &url,
        &[("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n")],
        &[],
    );
    git(&work, &["checkout", "-q", "-B", "change/2-y", &parent]);
    write(&work, "b.txt", "B\n");
    git(&work, &["commit", "-qam", "feat: y\n\nChange-Scope: b.txt"]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/change/2-y"]);
    // The bound job of change/2-y sees what it is built on.
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/2-y", &head).expect("checks out");
    fetch_branch(job.path(), &url, "", "change/1-x").expect("fetches");
    let seen = built_on_head(job.path(), "change/1-x", &head).expect("merge base");
    assert_eq!(seen, parent);
    // main moves; change/1-x restacks onto it, then merges as a squash.
    git(&work, &["checkout", "-q", "main"]);
    write(&work, "c.txt", "C\n");
    git(&work, &["add", "c.txt"]);
    git(&work, &["commit", "-qm", "chore: c"]);
    git(&work, &["checkout", "-q", "change/1-x"]);
    git(&work, &["rebase", "-q", "main"]);
    let restacked = git(&work, &["rev-parse", "HEAD"]);
    assert_ne!(restacked, parent);
    git(&work, &["checkout", "-q", "main"]);
    write(&work, "a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n");
    git(&work, &["commit", "-qam", "feat: x (!1)"]);
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/main"]);
    let job = tempfile::tempdir().expect("tmp");
    checkout(job.path(), &url, "", "main", "change/2-y", &head).expect("checks out");
    let r = restack(
        job.path(),
        &url,
        "",
        "main",
        "change/2-y",
        &head,
        Some(&seen),
        &["*.md"],
    )
    .expect("restacks");
    let to = r.to.expect("pushed");
    let message = git(job.path(), &["log", "-1", "--format=%B", &to]);
    assert!(message.starts_with("feat: y"), "{message}");
}
