//! `pigeon-sentinel` end to end; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

const POLICY: &str = "description: f starts with ok.\ntarget: { file: f.md }\nrules:\n  - line: { at: 1, matches: '^ok$' }\n";

fn sentinel(args: &[&str], root: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pigeon-sentinel"));
    cmd.args(args);
    if let Some(r) = root {
        cmd.arg("--root").arg(r);
    }
    for var in [
        "PR_TITLE",
        "PR_BODY",
        "PR_NUMBER",
        "PR_FILES",
        "PR_BRANCH",
        "PR_SCOPE",
    ] {
        cmd.env_remove(var);
    }
    cmd.output().expect("runs")
}

fn fixture(policy: &str, f: Option<&str>) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    fs::create_dir_all(root.path().join(".config/policies")).expect("mkdir");
    fs::write(root.path().join(".config/policies/p.yaml"), policy).expect("write");
    if let Some(text) = f {
        fs::write(root.path().join("f.md"), text).expect("write");
    }
    root
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn it_names_its_version() {
    let o = sentinel(&["--version"], None);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        stdout(&o).trim(),
        format!("pigeon-sentinel {}", sentinel::VERSION)
    );
}

#[test]
fn a_tree_that_holds_exits_0_and_reports_as_json() {
    let root = fixture(POLICY, Some("ok\n"));
    let o = sentinel(&["check", "--json"], Some(root.path()));
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("json");
    assert_eq!(v["ran"], 1);
    assert_eq!(v["violations"], serde_json::json!([]));
}

#[test]
fn a_violation_exits_1_and_names_it() {
    let root = fixture(POLICY, Some("no\n"));
    let o = sentinel(&["check"], Some(root.path()));
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).contains("f.md:1: p: line does not match `^ok$`"),
        "{}",
        stdout(&o)
    );
    let o = sentinel(&["check", "--json"], Some(root.path()));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("json");
    assert_eq!(v["violations"].as_array().map(Vec::len), Some(1));
}

#[test]
fn an_invalid_policy_or_argument_exits_2() {
    let root = fixture(
        "description: x\ntarget: { file: f.md }\nrules:\n  - nope: {}\n",
        None,
    );
    let o = sentinel(&["check"], Some(root.path()));
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("p.yaml"));
    assert_eq!(sentinel(&["judge"], None).status.code(), Some(2));
}

#[test]
fn a_policy_directory_can_be_named_apart_from_the_root() {
    let policies = fixture(POLICY, None);
    let tree = TempDir::new().expect("tempdir");
    fs::write(tree.path().join("f.md"), "ok\n").expect("write");
    let dir = policies.path().join(".config/policies");
    let o = sentinel(
        &["check", "--policies", dir.to_str().expect("utf-8")],
        Some(tree.path()),
    );
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
}

#[test]
fn describe_lists_each_policy_and_its_target() {
    let root = TempDir::new().expect("tempdir");
    let dir = root.path().join(".config/policies");
    fs::create_dir_all(&dir).expect("mkdir");
    for (name, target) in [
        ("a-file", "{ file: f.md }"),
        ("b-files", "{ files: [x.md, y.md] }"),
        ("c-dir", "{ dir: docs }"),
        ("d-tree", "tree"),
        ("e-pr", "pull-request"),
        ("f-change", "change"),
    ] {
        let rule = if target == "tree" {
            "  - find: { match: '^x$', forbid_content: 'y' }"
        } else if target.starts_with('{') {
            "  - line: { at: 1, matches: '^ok$' }"
        } else {
            "  - field: { name: title, matches: '^x' }"
        };
        fs::write(
            dir.join(format!("{name}.yaml")),
            format!("description: d.\ntarget: {target}\nrules:\n{rule}\n"),
        )
        .expect("write");
    }
    let o = sentinel(&["describe"], Some(root.path()));
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("json");
    let got: Vec<(String, String, Vec<String>)> = v
        .as_array()
        .expect("a list")
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap_or_default().to_string(),
                p["target"]["kind"].as_str().unwrap_or_default().to_string(),
                p["target"]["paths"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect(),
            )
        })
        .collect();
    let want = |n: &str, k: &str, p: &[&str]| {
        (
            n.to_string(),
            k.to_string(),
            p.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        got,
        [
            want("a-file", "file", &["f.md"]),
            want("b-files", "files", &["x.md", "y.md"]),
            want("c-dir", "dir", &["docs"]),
            want("d-tree", "tree", &[]),
            want("e-pr", "pull-request", &[]),
            want("f-change", "change", &[]),
        ]
    );
}

#[test]
fn describe_refuses_an_invalid_policy() {
    let root = fixture(
        "description: x\ntarget: { file: f.md }\nrules:\n  - nope: {}\n",
        None,
    );
    assert_eq!(
        sentinel(&["describe"], Some(root.path())).status.code(),
        Some(2)
    );
}
