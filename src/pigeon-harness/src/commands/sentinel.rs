//! `pigeon sentinel`: runs the `pigeon-sentinel` binary; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::INVALID;
use crate::cli::{CheckArgs, SentinelCommand};

/// The binary's name.
const BINARY: &str = "pigeon-sentinel";

/// Runs a `pigeon sentinel` command and returns its exit code.
#[must_use]
pub fn run(command: SentinelCommand) -> i32 {
    match command {
        SentinelCommand::Check(args) => check(&args),
    }
}

/// `PIGEON_EVICTION`, else `PIGEON_SENTINEL` with a warning, else `pigeon-sentinel` beside this
/// executable or in its parent directory, else on `PATH`. This feature links no backplane, so
/// it reads both names itself; see `backplane::names`.
fn locate() -> PathBuf {
    let set = |k: &str| env::var_os(k).filter(|p| !p.is_empty());
    if let Some(p) = set("PIGEON_EVICTION") {
        return PathBuf::from(p);
    }
    if let Some(p) = set("PIGEON_SENTINEL") {
        eprintln!("warning: PIGEON_SENTINEL is deprecated; set PIGEON_EVICTION");
        return PathBuf::from(p);
    }
    if let Ok(exe) = env::current_exe() {
        let here = exe.parent();
        for dir in here.into_iter().chain(here.and_then(Path::parent)) {
            let candidate = dir.join(BINARY);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(BINARY)
}

/// `pigeon-sentinel check`'s arguments for `args`.
fn argv(args: &CheckArgs) -> Vec<String> {
    let mut out = vec![
        "check".to_string(),
        "--root".into(),
        args.root.display().to_string(),
    ];
    if let Some(p) = &args.policies {
        out.extend(["--policies".into(), p.display().to_string()]);
    }
    if args.pull_request {
        out.push("--pull-request".into());
    }
    out
}

fn check(args: &CheckArgs) -> i32 {
    let binary = locate();
    match Command::new(&binary).args(argv(args)).status() {
        Ok(status) => status.code().unwrap_or(INVALID),
        Err(e) => {
            eprintln!("sentinel: {} didn't run: {e}", binary.display());
            INVALID
        }
    }
}
