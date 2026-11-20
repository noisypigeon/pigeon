//! `pigeon dev <task>`: the task graph and its commands, run from the workspace root;
//! see .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §4.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A task: its commands, each run with `sh -ec` from the workspace root after its dependencies.
#[derive(Debug)]
pub struct Task {
    /// Its name.
    pub name: &'static str,
    /// What it does.
    pub about: &'static str,
    /// The tasks that run first, in order.
    pub depends: &'static [&'static str],
    /// Its commands, in order.
    pub run: &'static [&'static str],
}

const POLICY: &str = "cargo run -q --locked -p pigeon-harness -- run policy.check --root .";

/// Every task.
pub const TASKS: &[Task] = &[
    Task {
        name: "build",
        about: "Build the workspace",
        depends: &[],
        run: &["cargo build --workspace --locked"],
    },
    Task {
        name: "test",
        about: "Run every test",
        depends: &[],
        run: &["cargo test --workspace --locked"],
    },
    Task {
        name: "fmt",
        about: "Format the workspace",
        depends: &[],
        run: &["cargo fmt --all"],
    },
    Task {
        name: "fmt-check",
        about: "Check the workspace is formatted",
        depends: &[],
        run: &["cargo fmt --all -- --check"],
    },
    Task {
        name: "lint",
        about: "clippy, every target, -D warnings",
        depends: &[],
        run: &["cargo clippy --workspace --all-targets --locked -- -D warnings"],
    },
    Task {
        name: "features",
        about: "clippy on pigeon with each command group's feature alone, and the Worker for wasm",
        depends: &[],
        run: &[
            "cargo clippy -p pigeon-harness --all-targets --locked --no-default-features -- -D warnings",
            "cargo clippy -p pigeon-harness --all-targets --locked --no-default-features --features eviction -- -D warnings",
            "cargo clippy -p pigeon-harness --all-targets --locked --no-default-features --features infra -- -D warnings",
            "cargo clippy -p pigeon-harness --all-targets --locked --no-default-features --features agent -- -D warnings",
            "cargo clippy -p pigeon-harness --all-targets --locked --no-default-features --features signing -- -D warnings",
            "cargo clippy -p pigeon-harness --all-targets --locked --no-default-features --features session -- -D warnings",
            "cargo clippy -p pigeon-agent --lib --locked --no-default-features --features worker --target wasm32-unknown-unknown -- -D warnings",
        ],
    },
    Task {
        name: "rust",
        about: "Every Rust check",
        depends: &["fmt-check", "lint", "features", "test"],
        run: &[],
    },
    Task {
        name: "policy-build",
        about: "Build the judge, which policy.check runs",
        depends: &[],
        run: &["cargo build -q --locked -p pigeon-eviction --bins"],
    },
    Task {
        name: "policy",
        about: "policy.check on the working tree",
        depends: &["policy-build"],
        run: &[POLICY],
    },
    Task {
        name: "policy-pr",
        about: "policy.check with the pull-request policy, from PR_TITLE, PR_BODY, PR_NUMBER and PR_FILES",
        depends: &["policy-build"],
        run: &[concat!(
            "cargo run -q --locked -p pigeon-harness -- run policy.check --root .",
            r#" --text "title=$PR_TITLE" --text "body=$PR_BODY" --text "number=$PR_NUMBER" --text "files=$PR_FILES""#
        )],
    },
    Task {
        name: "policy-change",
        about: "policy.check on this branch as a change, against CHANGE_BASE (default origin/main)",
        depends: &["policy-build"],
        run: &[r#"base="${CHANGE_BASE:-origin/main}"
branch="$(git rev-parse --abbrev-ref HEAD)"
first="$(git rev-list --reverse "$base"..HEAD | head -n 1)"
number="$(printf '%s' "$branch" | sed -n 's#^change/\([0-9]*\)-.*#\1#p')"
cargo run -q --locked -p pigeon-harness -- run policy.check --root . \
  --text "branch=$branch" \
  --text "number=${number:-0}" \
  --text "title=$(git log -1 --format=%s "$first")" \
  --text "body=$(git log -1 --format=%b "$first")" \
  --text "scope=$(git log -1 --format='%(trailers:key=Change-Scope,valueonly)' "$first")" \
  --text "test_reason=$(git log -1 --format='%(trailers:key=Test-Reason,valueonly)' "$first")" \
  --text "diff=$(git diff --unified=0 "$base"...HEAD -- '*.rs')" \
  --text "files=$(git diff --name-only "$base"...HEAD)"
"#],
    },
    Task {
        name: "instance",
        about: "Validate the instance manifest",
        depends: &[],
        run: &["cargo run -q --locked -p pigeon-harness -- instance --root ."],
    },
    Task {
        name: "capabilities",
        about: "Check every capability manifest's shape",
        depends: &[],
        run: &["cargo test -q --locked -p pigeon-core --lib capability::"],
    },
    Task {
        name: "ci",
        about: "Every check",
        depends: &["rust", "policy", "instance"],
        run: &[],
    },
    Task {
        name: "ci-scoped",
        about: "Only the checks this branch's paths need, against CHANGE_BASE (default origin/main)",
        depends: &[],
        run: &[
            r#"cargo run -q --locked -p pigeon-harness -- dev scope --root . --base "${CHANGE_BASE:-origin/main}" --run"#,
        ],
    },
];

/// The task named `name`.
#[must_use]
pub fn find(name: &str) -> Option<&'static Task> {
    TASKS.iter().find(|t| t.name == name)
}

/// The names `pigeon dev` takes.
#[must_use]
pub fn names() -> Vec<clap::builder::PossibleValue> {
    TASKS
        .iter()
        .map(|t| clap::builder::PossibleValue::new(t.name).help(t.about))
        .collect()
}

/// The tasks `name` runs, dependencies first, each once.
///
/// # Errors
/// An unknown task, or a dependency cycle.
pub fn plan(name: &str) -> Result<Vec<&'static Task>, String> {
    fn visit(
        name: &str,
        out: &mut Vec<&'static Task>,
        path: &mut Vec<&'static str>,
    ) -> Result<(), String> {
        let task = find(name).ok_or_else(|| format!("no task `{name}`"))?;
        if out.iter().any(|t| t.name == task.name) {
            return Ok(());
        }
        if path.contains(&task.name) {
            return Err(format!("`{}` depends on itself", task.name));
        }
        path.push(task.name);
        for dep in task.depends {
            visit(dep, out, path)?;
        }
        path.pop();
        out.push(task);
        Ok(())
    }
    let mut out = Vec::new();
    visit(name, &mut out, &mut Vec::new())?;
    Ok(out)
}

/// The nearest directory from `start` up whose `Cargo.toml` declares a workspace.
#[must_use]
pub fn workspace_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .ok()
            .filter(|text| text.lines().any(|l| l.trim() == "[workspace]"))
            .map(|_| dir.to_path_buf())
    })
}

/// Runs `name` and its dependencies from `root`, stopping at the first command that fails;
/// with `dry_run`, prints each command instead. Returns the exit code.
#[must_use]
pub fn run(root: &Path, name: &str, dry_run: bool) -> i32 {
    let tasks = match plan(name) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("dev: {e}");
            return 2;
        }
    };
    for task in tasks {
        for command in task.run {
            if dry_run {
                println!("{}: {command}", task.name);
                continue;
            }
            eprintln!("[{}] $ {}", task.name, command.lines().next().unwrap_or(""));
            let status = Command::new("sh")
                .arg("-ec")
                .arg(command)
                .current_dir(root)
                .status();
            match status {
                Ok(s) if s.success() => {}
                Ok(s) => {
                    eprintln!("dev: `{}` failed ({s})", task.name);
                    return s.code().filter(|&c| c != 0).unwrap_or(1);
                }
                Err(e) => {
                    eprintln!("dev: `{}`: sh: {e}", task.name);
                    return 2;
                }
            }
        }
    }
    0
}

/// Prints each task, what it depends on and what it does.
pub fn list() {
    for t in TASKS {
        let depends = if t.depends.is_empty() {
            String::new()
        } else {
            format!(" (after {})", t.depends.join(", "))
        };
        println!("{:<14} {}{depends}", t.name, t.about);
    }
}
