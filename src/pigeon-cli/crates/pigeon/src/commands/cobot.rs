//! `pigeon cobot`; see `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`,
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` and
//! `.claude/decisions/2026-11-02-development-sentinel-policy-cobot-actions.md`.

use std::env;
use std::fs;
use std::io::Read;

use cobot::release::{self, Config};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{
    CobotCommand, CobotPlan, CobotRun, ReleaseArgs, SecretsCommand, SecretsKey, SecretsUnit,
    TagsArgs,
};

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
        CobotCommand::Release(args) => return release(&args),
        CobotCommand::Tags(args) => return tags(&args),
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
        workload_secrets_passphrase: env::var(cobot::secrets::WORKLOAD_PASSPHRASE_VAR).ok(),
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

fn secrets_pass(args: &SecretsUnit) -> Result<String, cobot::Error> {
    cobot::secrets_passphrase_for(
        &args.unit,
        env::var(cobot::secrets::PASSPHRASE_VAR).ok(),
        env::var(cobot::secrets::WORKLOAD_PASSPHRASE_VAR).ok(),
    )
}

fn secrets_path(args: &SecretsUnit) -> std::path::PathBuf {
    args.root
        .join(args.unit.trim_end_matches('/'))
        .join(cobot::secrets::FILE)
}

fn secrets_set(args: &SecretsKey) -> Result<(), cobot::Error> {
    let pass = secrets_pass(&args.at)?;
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
    let pass = secrets_pass(args)?;
    let path = secrets_path(args);
    let values = cobot::secrets::decrypt_all(&path, &pass)?;
    for key in values.keys() {
        println!("{key}: decrypts");
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(result: Result<T, cobot::Error>) -> i32 {
    match result.map(|v| serde_json::to_string(&v)) {
        Ok(Ok(json)) => {
            println!("{json}");
            SUCCESS
        }
        Ok(Err(e)) => {
            eprintln!("cobot: {e}");
            INVALID
        }
        Err(e) => {
            eprintln!("cobot: {e}");
            INVALID
        }
    }
}

fn tags(args: &TagsArgs) -> i32 {
    let path = args
        .config
        .clone()
        .unwrap_or_else(|| args.root.join(".config/releases.yaml"));
    print_json(Config::load(&path).and_then(|config| release::tags(&args.root, &config)))
}

fn release(args: &ReleaseArgs) -> i32 {
    let path = args
        .config
        .clone()
        .unwrap_or_else(|| args.root.join(".config/releases.yaml"));
    print_json(Config::load(&path).and_then(|config| {
        release::run(
            &args.root,
            &config,
            args.from.as_deref(),
            &args.to,
            args.notes_dir.as_deref(),
            args.dry_run,
        )
    }))
}
