//! `pigeon dev`: the task graph in Rust, its only source;
//! see .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §4.

use std::path::Path;

use assert_cmd::Command;
use pigeon::dev::{TASKS, find, plan, workspace_root};

fn pigeon() -> Command {
    Command::cargo_bin("pigeon").expect("binary")
}

fn root() -> std::path::PathBuf {
    workspace_root(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("a workspace above the crate")
}

#[test]
fn every_name_is_unique_and_dependencies_use_new_names() {
    let mut seen = std::collections::BTreeSet::new();
    for task in TASKS {
        assert!(!task.about.is_empty(), "{} says what it does", task.name);
        assert!(
            !task.run.is_empty() || !task.depends.is_empty(),
            "{} does something",
            task.name
        );
        for name in std::iter::once(task.name).chain(task.old) {
            assert!(seen.insert(name), "`{name}` names two tasks");
        }
        for dep in task.depends {
            let found = find(dep).unwrap_or_else(|| panic!("{}: no task `{dep}`", task.name));
            assert_eq!(
                found.name, *dep,
                "{} names `{dep}` by its old name",
                task.name
            );
        }
    }
}

#[test]
fn no_dev_command_names_an_old_spelling() {
    for task in TASKS {
        for command in task.run {
            for old in [
                "-- backplane",
                "-- cobot",
                "-- sentinel",
                "--features sentinel",
            ] {
                assert!(!command.contains(old), "{}: {command}", task.name);
            }
        }
    }
}

#[test]
fn a_plan_runs_each_dependency_once_before_its_dependents() {
    let names =
        |task: &str| -> Vec<&str> { plan(task).expect("plan").iter().map(|t| t.name).collect() };
    assert_eq!(
        names("ci"),
        [
            "fmt-check",
            "lint",
            "features",
            "test",
            "rust",
            "policy-build",
            "policy",
            "instance",
            "ci"
        ]
    );
    assert_eq!(names("policy-change"), ["policy-build", "policy-change"]);
    assert_eq!(names("build"), ["build"]);
    for task in TASKS {
        let order = names(task.name);
        assert_eq!(order.last(), Some(&task.name));
        for dep in task.depends {
            let at = |n: &str| order.iter().position(|o| *o == n).expect("planned");
            assert!(at(dep) < at(task.name), "{dep} before {}", task.name);
        }
        let mut unique = order.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), order.len(), "{} runs a task twice", task.name);
    }
    assert!(plan("nope").is_err());
}

#[test]
fn old_names_find_their_tasks() {
    for (old, new) in [
        ("sentinel", "policy"),
        ("sentinel-pr", "policy-pr"),
        ("sentinel-change", "policy-change"),
        ("sentinel-build", "policy-build"),
        ("backplane", "instance"),
    ] {
        assert_eq!(find(old).expect(old).name, new);
    }
}

#[test]
fn dry_run_prints_the_plan_in_order() {
    let out = pigeon()
        .args(["dev", "rust", "--dry-run"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    let tasks: Vec<_> = stdout
        .lines()
        .filter_map(|l| l.split_once(": ").map(|(t, _)| t))
        .collect();
    let mut seen: Vec<&str> = Vec::new();
    for t in tasks {
        if seen.last() != Some(&t) {
            seen.push(t);
        }
    }
    assert_eq!(seen, ["fmt-check", "lint", "features", "test"]);
    assert!(stdout.contains("--features eviction"));
}

#[test]
fn an_old_task_name_runs_its_task_after_one_line_on_stderr() {
    let out = pigeon()
        .args(["dev", "backplane", "--dry-run"])
        .assert()
        .success();
    let o = out.get_output();
    assert_eq!(
        String::from_utf8_lossy(&o.stderr),
        "pigeon: `pigeon dev backplane` is now `pigeon dev instance`\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "instance: cargo run -q --locked -p pigeon -- instance --root .\n"
    );
}

#[test]
fn an_unknown_task_is_a_usage_error_and_old_names_stay_out_of_help() {
    pigeon().args(["dev", "nope"]).assert().code(2);
    pigeon().arg("dev").assert().code(2);
    let help = pigeon().args(["dev", "--help"]).assert().success();
    let text = String::from_utf8(help.get_output().stdout.clone()).expect("utf-8");
    assert!(text.contains("policy-change"), "{text}");
    for old in [
        "sentinel",
        "sentinel-build",
        "sentinel-pr",
        "sentinel-change",
        "backplane",
    ] {
        let listed = text.lines().any(|l| {
            l.trim()
                .strip_prefix("- ")
                .and_then(|v| v.split(':').next())
                == Some(old)
        });
        assert!(!listed, "{old} is listed: {text}");
    }
    assert!(
        text.lines()
            .any(|l| l.trim().starts_with("- policy-change:")),
        "{text}"
    );
}

#[test]
fn list_names_every_task() {
    let out = pigeon().args(["dev", "--list"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    for task in TASKS {
        assert!(
            text.lines().any(|l| l.starts_with(task.name)),
            "{}",
            task.name
        );
    }
}

#[test]
fn a_task_runs_from_the_root_and_its_failure_is_the_exit_code() {
    // A workspace whose member is missing, so cargo fails and the task stops there.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"missing\"]\n",
    )
    .expect("manifest");
    let out = pigeon()
        .current_dir(dir.path())
        .args(["dev", "fmt-check", "--root"])
        .arg(dir.path())
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("[fmt-check] $ cargo fmt --all -- --check"),
        "{stderr}"
    );
    assert!(stderr.contains("dev: `fmt-check` failed"), "{stderr}");
}

#[test]
fn without_root_it_finds_the_workspace_above_the_working_directory() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(workspace_root(crate_dir), Some(root()));
    assert!(root().join("Cargo.toml").is_file());
    let dir = tempfile::tempdir().expect("tempdir");
    pigeon()
        .current_dir(dir.path())
        .args(["dev", "build"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("no workspace Cargo.toml above"));
}
