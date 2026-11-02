//! `pigeon cobot`; see `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`
//! and `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md`.

use std::env;
use std::fs;
use std::io::Read;

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{CobotCommand, CobotPlan, CobotRun, SecretsCommand, SecretsKey, SecretsUnit};

/// Runs a `pigeon cobot` command and returns its exit code.
#[must_use]
pub fn run(command: CobotCommand) -> i32 {
    let result = match command {
        CobotCommand::Validate(args) => cobot::validate(&args.root, &args.providers_dir())
            .map(|units| println!("cobot: {} unit(s) valid", units.len())),
        CobotCommand::Render(args) => render(&args),
        CobotCommand::Plan(args) => plan(&args),
        CobotCommand::Apply(args) => apply(&args),
        CobotCommand::Lock(args) => cobot::lock(&args.root, &args.providers_dir())
            .map(|path| println!("cobot: wrote {}", path.display())),
        CobotCommand::Secrets(SecretsCommand::Set(args)) => secrets_set(&args),
        CobotCommand::Secrets(SecretsCommand::Check(args)) => secrets_check(&args),
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
    let providers = args.at.providers_dir();
    for unit in cobot::select(&args.at.root, &providers, &args.units)? {
        let work = cobot::render(&args.at.root, &providers, &unit, &args.state)?;
        println!("{}", work.display());
    }
    Ok(())
}

fn context(args: &CobotRun) -> Result<cobot::Context, cobot::Error> {
    Ok(cobot::Context {
        root: args.at.root.clone(),
        providers: args.at.providers_dir(),
        state: args.state.clone(),
        state_passphrase: cobot::passphrase(env::var(cobot::PASSPHRASE_VAR).ok())?,
        secrets_passphrase: env::var(cobot::secrets::PASSPHRASE_VAR).ok(),
        today: cobot::generations::today(),
    })
}

fn plan(args: &CobotPlan) -> Result<(), cobot::Error> {
    let ctx = context(&args.run)?;
    let units = cobot::select(&ctx.root, &ctx.providers, &args.run.units)?;
    let summary = cobot::summary(&cobot::plan(&ctx, &units)?);
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
    let ctx = context(args)?;
    let units = cobot::select(&ctx.root, &ctx.providers, &args.units)?;
    let plans = cobot::apply(&ctx, &units)?;
    print!("{}", cobot::summary(&plans));
    let applied: Vec<&str> = plans
        .iter()
        .filter(|p| p.has_changes())
        .map(|p| p.unit.as_str())
        .collect();
    let applied = if applied.is_empty() {
        "nothing".to_string()
    } else {
        applied.join(", ")
    };
    println!("cobot: applied {applied}");
    Ok(())
}

fn secrets_path(args: &SecretsUnit) -> std::path::PathBuf {
    args.root
        .join(args.unit.trim_end_matches('/'))
        .join(cobot::secrets::FILE)
}

fn secrets_set(args: &SecretsKey) -> Result<(), cobot::Error> {
    let pass = cobot::secrets_passphrase(env::var(cobot::secrets::PASSPHRASE_VAR).ok())?;
    let mut value = String::new();
    std::io::stdin()
        .read_to_string(&mut value)
        .map_err(|e| cobot::Error::Io {
            path: "<stdin>".into(),
            message: e.to_string(),
        })?;
    let value = value.trim_end_matches(['\n', '\r']);
    let path = secrets_path(&args.at);
    cobot::secrets::set(&path, &args.key, value, &pass)?;
    println!("cobot: set `{}` in {}", args.key, path.display());
    Ok(())
}

fn secrets_check(args: &SecretsUnit) -> Result<(), cobot::Error> {
    let pass = cobot::secrets_passphrase(env::var(cobot::secrets::PASSPHRASE_VAR).ok())?;
    let path = secrets_path(args);
    let values = cobot::secrets::decrypt_all(&path, &pass)?;
    for key in values.keys() {
        println!("{key}: decrypts");
    }
    Ok(())
}
