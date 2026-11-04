//! `pigeon cobot`; see `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`,
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` and
//! `.claude/decisions/2026-11-02-development-sentinel-policy-cobot-actions.md`.

use std::env;
use std::fs;
use std::io::Read;

use cobot::release::{self, Config};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{
    CobotApply, CobotCommand, CobotPlan, CobotRun, ReleaseArgs, SecretsCommand, SecretsKey,
    SecretsUnit, SitesBuild, SitesCommand, SitesPublish, TagsArgs,
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
        CobotCommand::Capabilities(args) => capabilities(&args.root),
        CobotCommand::Secrets(SecretsCommand::Set(args)) => secrets_set(&args),
        CobotCommand::Secrets(SecretsCommand::Check(args)) => secrets_check(&args),
        CobotCommand::Sites(SitesCommand::Build(args)) => sites_build(&args),
        CobotCommand::Sites(SitesCommand::Publish(args)) => sites_publish(&args),
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

fn capabilities(root: &std::path::Path) -> Result<(), cobot::Error> {
    let (catalogs, problems) = cobot::catalog::capabilities(root);
    if !problems.is_empty() {
        return Err(cobot::Error::Invalid(problems));
    }
    let json = serde_json::to_string_pretty(&catalogs).map_err(|e| cobot::Error::Io {
        path: root.to_path_buf(),
        message: e.to_string(),
    })?;
    println!("{json}");
    Ok(())
}

fn render(args: &CobotRun) -> Result<(), cobot::Error> {
    let providers = args.at.providers_dir();
    for unit in cobot::select(&args.at.root, &providers, &args.units)? {
        let state =
            cobot::session::render_dir(&args.at.root, &args.state.to_string_lossy(), |k| {
                env::var(k).ok()
            })?;
        let work = cobot::render(&args.at.root, &providers, &unit, &state)?;
        println!("{}", work.display());
    }
    Ok(())
}

fn sites_build(args: &SitesBuild) -> Result<(), cobot::Error> {
    let root = args.root.canonicalize().map_err(|e| cobot::Error::Io {
        path: args.root.clone(),
        message: e.to_string(),
    })?;
    let sites = cobot::sites::find(&root, &args.workloads)?;
    for b in cobot::sites::build(&root, &sites)? {
        let on: Vec<&str> = b
            .features
            .iter()
            .filter(|(_, on)| **on)
            .map(|(f, _)| f.as_str())
            .collect();
        println!(
            "cobot: built {} with {} [{}] into {}",
            b.site.unit,
            b.site.tag(),
            on.join(", "),
            b.public.display()
        );
    }
    if sites.is_empty() {
        println!("cobot: no static sites");
    }
    Ok(())
}

fn sites_publish(args: &SitesPublish) -> Result<(), cobot::Error> {
    let session = context(&args.run)?;
    let published = cobot::publish(&session.ctx, &args.run.units, args.checks)?;
    let mut failed = Vec::new();
    for p in &published {
        println!(
            "cobot: published {}: {} put, {} unchanged, {} deleted",
            p.unit,
            p.synced.put.len(),
            p.synced.unchanged,
            p.synced.deleted.len()
        );
        for k in &p.synced.put {
            println!("  put {k}");
        }
        for k in &p.synced.deleted {
            println!("  delete {k}");
        }
        for c in &p.checks {
            println!("  {} {}", if c.passed { "ok" } else { "FAILED" }, c.what);
            if !c.passed {
                failed.push(c.what.clone());
            }
        }
    }
    if published.is_empty() {
        println!("cobot: no static sites");
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(cobot::Error::Tool {
            command: "live checks".into(),
            message: failed.join("; "),
        })
    }
}

/// The run's context, with its state pulled; see [`cobot::session`].
fn context(args: &CobotRun) -> Result<cobot::session::Session, cobot::Error> {
    cobot::session::open(
        &args.at.root,
        &args.at.providers_dir(),
        &args.state.to_string_lossy(),
        |k| env::var(k).ok(),
    )
}

fn now() -> u64 {
    cobot::session::now()
}

fn plan(args: &CobotPlan) -> Result<(), cobot::Error> {
    let session = context(&args.run)?;
    let ctx = &session.ctx;
    let units = cobot::select(&ctx.root, &ctx.providers, &args.run.units)?;
    let plans = if args.store {
        let (stored, plans) = cobot::plan_stored(ctx, &units, now())?;
        session.push()?;
        println!(
            "cobot: stored plan {} for {} unit(s); apply it with --plan {} within 24 hours",
            stored.id,
            stored.units.len(),
            stored.id
        );
        plans
    } else {
        cobot::plan(ctx, &units)?
    };
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

fn apply(args: &CobotApply) -> Result<(), cobot::Error> {
    let session = context(&args.run)?;
    let ctx = &session.ctx;
    let applied = match &args.plan {
        Some(id) => cobot::apply_stored(ctx, id, now()),
        None => cobot::select(&ctx.root, &ctx.providers, &args.run.units)
            .and_then(|units| cobot::apply(ctx, &units)),
    };
    if let Some(written) = session.push()? {
        println!("cobot: wrote {written} state object(s) to R2");
    }
    let plans = applied?;
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
    let (_, values) = cobot::unit_secrets(
        &args.unit,
        &secrets_path(args),
        env::var(cobot::secrets::PASSPHRASE_VAR).ok(),
        env::var(cobot::secrets::WORKLOAD_PASSPHRASE_VAR).ok(),
    )?;
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
