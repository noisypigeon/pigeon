//! The command allowlist for benches.

use serde::{Deserialize, Serialize};

/// Tasks a bench may run, through `pigeon dev`.
pub const TASKS: [&str; 12] = [
    "ci-scoped",
    "build",
    "test",
    "fmt",
    "fmt-check",
    "lint",
    "rust",
    "features",
    "policy",
    "policy-build",
    "policy-change",
    "instance",
];
/// `cargo` subcommands a bench may run.
pub const CARGO: [&str; 7] = [
    "fmt", "check", "clippy", "build", "test", "tree", "metadata",
];
/// `cargo` arguments a bench refuses.
pub const CARGO_REFUSED: [&str; 4] = ["--config", "-Z", "--manifest-path", "--target-dir"];
/// `git` subcommands a bench may run.
pub const GIT: [&str; 3] = ["diff", "log", "status"];
/// Environment every bench command runs under.
pub const ENV: [(&str, &str); 4] = [
    ("CHANGE_BASE", "artifacts/main"),
    ("CARGO_NET_OFFLINE", "true"),
    ("CARGO_TERM_COLOR", "never"),
    ("RUST_BACKTRACE", "1"),
];
/// Most arguments a command may have.
pub const MAX_ARGS: usize = 64;
/// Most bytes one argument may have.
pub const MAX_ARG_BYTES: usize = 4096;

/// An allowed command: a `pigeon dev` task, a check by name, or a plain argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum Command {
    /// A `pigeon dev` task.
    Task {
        /// The task's name.
        name: String,
    },
    /// An argv to run as given.
    Argv {
        /// The program and its arguments.
        argv: Vec<String>,
    },
    /// A named check of the change, run by name.
    Check {
        /// The check's name.
        name: String,
    },
}

/// Why a command is not allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn r(s: impl Into<String>) -> Result<Command, Refusal> {
    Err(Refusal(s.into()))
}

/// Task `t`, run as `pigeon dev <t>`, if a bench may run it.
fn task(t: &str) -> Result<Command, Refusal> {
    if t == "ci" {
        r("bench: `pigeon dev ci` runs on the gate; use `bench ci`")
    } else if TASKS.contains(&t) {
        Ok(Command::Task { name: t.into() })
    } else {
        r(format!("bench: task `{t}` isn't allowed"))
    }
}

/// Judges an argv against the allowlist.
///
/// # Errors
///
/// Refuses any argv the allowlist does not name.
pub fn allow(argv: &[String]) -> Result<Command, Refusal> {
    if argv.is_empty() {
        return r("bench: an empty command");
    }
    if argv.len() > MAX_ARGS {
        return r("bench: more than 64 arguments");
    }
    if argv
        .iter()
        .any(|a| a.len() > MAX_ARG_BYTES || a.contains('\0'))
    {
        return r("bench: an argument is too long or holds NUL");
    }
    match argv[0].as_str() {
        "bench" => {
            let ok = argv.len() == 3
                && argv[1] == "check"
                && (1..=200).contains(&argv[2].len())
                && !argv[2].bytes().any(|b| b.is_ascii_control());
            if ok {
                Ok(Command::Check {
                    name: argv[2].clone(),
                })
            } else {
                r("bench: only `bench check <name>`")
            }
        }
        "cargo" => {
            if argv.len() < 2 {
                return r("bench: `cargo` needs a subcommand");
            }
            if !CARGO.contains(&argv[1].as_str()) {
                return r(format!("bench: `cargo {}` isn't allowed", argv[1]));
            }
            for a in &argv[2..] {
                let refused = CARGO_REFUSED
                    .iter()
                    .any(|x| a == x || a.starts_with(&format!("{x}=")))
                    || a.starts_with("-Z");
                if refused {
                    return r(format!("bench: `{a}` isn't allowed"));
                }
            }
            Ok(Command::Argv {
                argv: argv.to_vec(),
            })
        }
        "pigeon"
            if argv.get(1).is_some_and(|a| a == "dev")
                && argv.get(2).is_none_or(|a| a != "scope") =>
        {
            if argv.len() != 3 {
                return r("bench: only `pigeon dev <task>`");
            }
            task(&argv[2])
        }
        "pigeon" => {
            let ok = (argv.len() >= 3
                && ((argv[1] == "infra" && ["validate", "render"].contains(&argv[2].as_str()))
                    || (argv[1] == "dev" && argv[2] == "scope")))
                || (argv.len() >= 2 && argv[1] == "decisions");
            if ok {
                Ok(Command::Argv {
                    argv: argv.to_vec(),
                })
            } else {
                r(
                    "bench: only `pigeon infra validate|render`, `pigeon dev scope` and `pigeon decisions`",
                )
            }
        }
        "git" => {
            let ok = argv.len() >= 2
                && GIT.contains(&argv[1].as_str())
                && !argv[2..]
                    .iter()
                    .any(|a| a == "--output" || a.starts_with("--output=") || a == "--ext-diff");
            if ok {
                Ok(Command::Argv {
                    argv: argv.to_vec(),
                })
            } else {
                r("bench: only `git diff|log|status`, without `--output` or `--ext-diff`")
            }
        }
        other => r(format!("bench: `{other}` isn't allowed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(ToString::to_string).collect()
    }

    fn refused(s: &[&str]) -> String {
        allow(&v(s)).unwrap_err().to_string()
    }

    #[test]
    fn rule1_empty() {
        assert_eq!(refused(&[]), "bench: an empty command");
    }

    #[test]
    fn rule2_too_many() {
        let a = vec!["git".to_string(); 65];
        assert_eq!(allow(&a).unwrap_err().0, "bench: more than 64 arguments");
    }

    #[test]
    fn rule2_too_long() {
        let a = vec!["git".to_string(), "x".repeat(4097)];
        assert_eq!(
            allow(&a).unwrap_err().0,
            "bench: an argument is too long or holds NUL"
        );
        let a = vec!["git".to_string(), "a\0b".to_string()];
        assert_eq!(
            allow(&a).unwrap_err().0,
            "bench: an argument is too long or holds NUL"
        );
    }

    #[test]
    fn rule3_mise_is_refused() {
        for argv in [
            &["mise"][..],
            &["mise", "run", "fmt"],
            &["mise", "run", "ci"],
        ] {
            assert_eq!(refused(argv), "bench: `mise` isn't allowed");
        }
        for old in ["sentinel", "sentinel-build", "sentinel-change", "backplane"] {
            assert_eq!(
                refused(&["pigeon", "dev", old]),
                format!("bench: task `{old}` isn't allowed")
            );
        }
    }

    #[test]
    fn rule3_pigeon_dev_runs_the_same_tasks() {
        for t in TASKS {
            assert_eq!(
                allow(&v(&["pigeon", "dev", t])),
                Ok(Command::Task {
                    name: t.to_string()
                })
            );
        }
        assert_eq!(
            refused(&["pigeon", "dev", "ci"]),
            "bench: `pigeon dev ci` runs on the gate; use `bench ci`"
        );
        assert_eq!(
            refused(&["pigeon", "dev", "fmt", "x"]),
            "bench: only `pigeon dev <task>`"
        );
        assert_eq!(
            refused(&["pigeon", "dev", "deploy"]),
            "bench: task `deploy` isn't allowed"
        );
    }

    #[test]
    fn rule4_cargo_needs_subcommand() {
        assert_eq!(refused(&["cargo"]), "bench: `cargo` needs a subcommand");
    }

    #[test]
    fn rule4_cargo_subcommand() {
        assert_eq!(
            refused(&["cargo", "run"]),
            "bench: `cargo run` isn't allowed"
        );
    }

    #[test]
    fn rule4_cargo_dash_d_allowed() {
        let a = v(&[
            "cargo",
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]);
        assert_eq!(allow(&a), Ok(Command::Argv { argv: a.clone() }));
    }

    #[test]
    fn rule4_cargo_refused_args() {
        for a in [
            "--config",
            "--config=x",
            "-Zbuild-std",
            "--target-dir=/tmp",
            "--manifest-path",
        ] {
            assert_eq!(
                refused(&["cargo", "test", a]),
                format!("bench: `{a}` isn't allowed")
            );
        }
        assert_eq!(
            refused(&["cargo", "test", "--config", "x"]),
            "bench: `--config` isn't allowed"
        );
        assert_eq!(
            refused(&["cargo", "test", "--manifest-path", "/x"]),
            "bench: `--manifest-path` isn't allowed"
        );
    }

    #[test]
    fn rule5_pigeon() {
        let e =
            "bench: only `pigeon infra validate|render`, `pigeon dev scope` and `pigeon decisions`";
        assert_eq!(refused(&["pigeon", "infra", "plan"]), e);
        assert_eq!(refused(&["pigeon"]), e);
        assert_eq!(refused(&["pigeon", "run"]), e);
        for old in [
            &["pigeon", "cobot", "validate"][..],
            &["pigeon", "backplane", "decisions", "x"],
        ] {
            assert_eq!(refused(old), e);
        }
        for ok in [
            v(&["pigeon", "dev", "scope", "--json"]),
            v(&["pigeon", "infra", "render", "--state", "s"]),
            v(&["pigeon", "decisions", "x"]),
        ] {
            assert_eq!(allow(&ok), Ok(Command::Argv { argv: ok.clone() }));
        }
    }

    #[test]
    fn rule6_git() {
        let e = "bench: only `git diff|log|status`, without `--output` or `--ext-diff`";
        assert_eq!(refused(&["git", "diff", "--output=/tmp/x"]), e);
        assert_eq!(refused(&["git", "diff", "--output", "x"]), e);
        assert_eq!(refused(&["git", "diff", "--ext-diff"]), e);
        assert_eq!(refused(&["git", "push"]), e);
        assert_eq!(refused(&["git"]), e);
        let a = v(&["git", "log", "-3"]);
        assert_eq!(allow(&a), Ok(Command::Argv { argv: a.clone() }));
    }

    #[test]
    fn rule7_other() {
        assert_eq!(refused(&["sh", "-c", "x"]), "bench: `sh` isn't allowed");
    }

    #[test]
    fn display_writes_text() {
        assert_eq!(Refusal("x".to_string()).to_string(), "x");
    }

    #[test]
    fn rule3b_bench_check() {
        assert_eq!(
            allow(&v(&["bench", "check", "test pigeon-backplane: lib"])),
            Ok(Command::Check {
                name: "test pigeon-backplane: lib".to_string()
            })
        );
    }

    #[test]
    fn rule3b_bench_refused() {
        let e = "bench: only `bench check <name>`";
        assert_eq!(refused(&["bench", "check"]), e);
        assert_eq!(refused(&["bench", "check", ""]), e);
        assert_eq!(refused(&["bench", "check", &"x".repeat(201)]), e);
        assert_eq!(refused(&["bench", "check", "a\nb"]), e);
        assert_eq!(refused(&["bench", "run", "x"]), e);
        assert!(allow(&v(&["bench", "check", &"x".repeat(200)])).is_ok());
    }

    #[test]
    fn command_serializes() {
        let t = Command::Task {
            name: "ci-scoped".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"kind":"task","name":"ci-scoped"}"#
        );
        let a = Command::Argv {
            argv: v(&["git", "log"]),
        };
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"{"kind":"argv","argv":["git","log"]}"#
        );
        let c = Command::Check {
            name: "fmt".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"kind":"check","name":"fmt"}"#
        );
    }
}
