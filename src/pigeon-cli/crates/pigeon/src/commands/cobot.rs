//! `pigeon cobot`; see `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`.

use std::env;
use std::fs;

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{CobotCommand, CobotPlan, CobotRun};

/// Runs a `pigeon cobot` command and returns its exit code.
#[must_use]
pub fn run(command: CobotCommand) -> i32 {
    let result = match command {
        CobotCommand::Validate(args) => cobot::validate(&args.root).map(|units| {
            println!("cobot: {} unit(s) valid", units.len());
        }),
        CobotCommand::Render(args) => render(&args),
        CobotCommand::Plan(args) => plan(&args),
        CobotCommand::Apply(args) => apply(&args),
    };
    match result {
        Ok(()) => SUCCESS,
        Err(e @ cobot::Error::Invalid(_)) => {
            eprintln!("cobot: {e}");
            VIOLATION
        }
        Err(e) => {
            eprintln!("cobot: {e}");
            INVALID
        }
    }
}

fn render(args: &CobotRun) -> Result<(), cobot::Error> {
    for unit in cobot::select(&args.root, &args.units)? {
        let work = cobot::render(&args.root, &unit, &args.state)?;
        println!("{}", work.display());
    }
    Ok(())
}

fn passphrase() -> Result<String, cobot::Error> {
    cobot::passphrase(env::var(cobot::PASSPHRASE_VAR).ok())
}

fn plan(args: &CobotPlan) -> Result<(), cobot::Error> {
    let passphrase = passphrase()?;
    let units = cobot::select(&args.run.root, &args.run.units)?;
    let plans = cobot::plan(&args.run.root, &units, &args.run.state, &passphrase)?;
    let summary = cobot::summary(&plans);
    print!("{summary}");
    if let Some(path) = &args.summary {
        fs::write(path, &summary).map_err(|e| cobot::Error::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
    }
    Ok(())
}

fn apply(args: &CobotRun) -> Result<(), cobot::Error> {
    let passphrase = passphrase()?;
    let units = cobot::select(&args.root, &args.units)?;
    let plans = cobot::apply(&args.root, &units, &args.state, &passphrase)?;
    print!("{}", cobot::summary(&plans));
    let applied: Vec<&str> = plans
        .iter()
        .filter(|p| p.has_changes())
        .map(|p| p.unit.as_str())
        .collect();
    println!(
        "cobot: applied {}",
        if applied.is_empty() {
            "nothing".to_string()
        } else {
            applied.join(", ")
        }
    );
    Ok(())
}
