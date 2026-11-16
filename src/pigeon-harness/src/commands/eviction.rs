//! `pigeon eviction`: runs the `pigeon-sentinel` binary; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::INVALID;
use crate::cli::{CheckArgs, EvictionCommand};

/// The binary's name, and its new one, found first; see
/// `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §3.
const BINARY: &str = "pigeon-sentinel";
const BINARIES: [&str; 2] = ["pigeon-eviction", BINARY];

/// Runs a `pigeon eviction` command and returns its exit code.
#[must_use]
pub fn run(command: EvictionCommand) -> i32 {
    match command {
        EvictionCommand::Check(args) => check(&args),
    }
}

/// `PIGEON_EVICTION`, else either of [`BINARIES`] beside this executable, in its parent directory
/// or on `PATH`, else [`BINARY`].
fn locate() -> PathBuf {
    let set = |k: &str| env::var_os(k).filter(|p| !p.is_empty());
    if let Some(p) = set("PIGEON_EVICTION") {
        return PathBuf::from(p);
    }
    let exe = env::current_exe().ok();
    let here = exe.as_deref().and_then(Path::parent);
    let dirs: Vec<PathBuf> = here
        .into_iter()
        .chain(here.and_then(Path::parent))
        .map(Path::to_path_buf)
        .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default()))
        .collect();
    dirs.iter()
        .find_map(|d| BINARIES.iter().map(|b| d.join(b)).find(|c| c.is_file()))
        .unwrap_or_else(|| PathBuf::from(BINARY))
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
