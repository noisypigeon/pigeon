//! `pigeon infra`; see `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`,
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` and
//! `.claude/decisions/2026-11-02-development-sentinel-policy-cobot-actions.md`.

use agent::cobot::release::LoadConfig;
use agent::cobot::scope::ReadTestMap;
use std::env;
use std::fs;
use std::io::Read;

use agent::cobot::release::{self, Config};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{
    GitCommand, GitMirror, GitRun, InfraApply, InfraOp, InfraPlan, InfraRoot, InfraRun, InfraShow,
    ReleaseArgs, ScopeArgs, SecretsCommand, SecretsKey, SecretsUnit, SitesBuild, SitesCommand,
    SitesPublish, TagsArgs,
};

/// Runs a `pigeon infra` command and returns its exit code.
#[must_use]
pub fn run(command: InfraOp) -> i32 {
    let result = match command {
        InfraOp::Validate(args) => {
            { agent::cobot::validate_in(args.instance_root(), &args.root, &args.providers_dir()) }
                .map(|units| println!("cobot: {} unit(s) valid", units.len()))
        }
        InfraOp::Render(args) => render(&args),
        InfraOp::Plan(args) => plan(&args),
        InfraOp::Apply(args) => apply(&args),
        InfraOp::Show(args) => show(&args),
        InfraOp::Lock(args) => agent::cobot::lock(&args.root, &args.providers_dir())
            .map(|path| println!("cobot: wrote {}", path.display())),
        InfraOp::Capabilities(args) => capabilities(&args.root),
        InfraOp::Secrets(SecretsCommand::Set(args)) => secrets_set(&args),
        InfraOp::Secrets(SecretsCommand::Check(args)) => secrets_check(&args),
        InfraOp::Sites(SitesCommand::Build(args)) => sites_build(&args),
        InfraOp::Sites(SitesCommand::Publish(args)) => sites_publish(&args),
        InfraOp::Git(command) => git(command),
        InfraOp::Workers(command) => super::workers::run(command),
        InfraOp::Scope(args) => return scope(&args),
        InfraOp::Release(args) => return release(&args),
        InfraOp::Tags(args) => return tags(&args),
    };
    match result {
        Ok(()) => SUCCESS,
        Err(e @ agent::cobot::Error::Invalid(_)) => {
            eprintln!("cobot: {e}");
            VIOLATION
        }
        Err(e) => {
            eprintln!("cobot: {e}");
            INVALID
        }
    }
}

fn capabilities(root: &std::path::Path) -> Result<(), agent::cobot::Error> {
    let (catalogs, problems) = agent::cobot::catalog::capabilities(root);
    if !problems.is_empty() {
        return Err(agent::cobot::Error::Invalid(problems));
    }
    let json = serde_json::to_string_pretty(&catalogs).map_err(|e| agent::cobot::Error::Io {
        path: root.to_path_buf(),
        message: e.to_string(),
    })?;
    println!("{json}");
    Ok(())
}

fn render(args: &InfraRun) -> Result<(), agent::cobot::Error> {
    let providers = args.at.providers_dir();
    let instance = args.at.instance_root();
    for unit in agent::cobot::select_in(instance, &args.at.root, &providers, &args.units)? {
        let state =
            agent::cobot::session::render_dir(instance, &args.state.to_string_lossy(), |k| {
                env::var(k).ok()
            })?;
        let work = agent::cobot::render(instance, &providers, &unit, &state)?;
        println!("{}", work.display());
    }
    Ok(())
}

/// Each `NAME=FILE`, read as JSON.
fn data_files(
    args: &[String],
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, agent::cobot::Error> {
    let mut out = std::collections::BTreeMap::new();
    for arg in args {
        let fail = |message: String| agent::cobot::Error::Io {
            path: arg.into(),
            message,
        };
        let (name, file) = arg
            .split_once('=')
            .ok_or_else(|| fail("--data takes NAME=FILE".into()))?;
        if !agent::cobot::sites::is_data_name(name) {
            return Err(fail(
                "a data name is lowercase letters, digits and dashes".into(),
            ));
        }
        let text = fs::read_to_string(file).map_err(|e| fail(e.to_string()))?;
        let value = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
        out.insert(name.to_string(), value);
    }
    Ok(out)
}

fn sites_build(args: &SitesBuild) -> Result<(), agent::cobot::Error> {
    let canonical = |p: &std::path::Path| {
        p.canonicalize().map_err(|e| agent::cobot::Error::Io {
            path: p.to_path_buf(),
            message: e.to_string(),
        })
    };
    let platform = canonical(&args.root)?;
    let root = canonical(args.instance.as_deref().unwrap_or(&args.root))?;
    let data = data_files(&args.data)?;
    let sites = agent::cobot::sites::find_in(&root, &platform, &args.workloads)?;
    for b in agent::cobot::sites::build_in(&root, &platform, &sites, &data)? {
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

fn sites_publish(args: &SitesPublish) -> Result<(), agent::cobot::Error> {
    let session = context(&args.run)?;
    let data = data_files(&args.data)?;
    let published = agent::cobot::publish(&session.ctx, &args.run.units, args.checks, &data)?;
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
        Err(agent::cobot::Error::Tool {
            command: "live checks".into(),
            message: failed.join("; "),
        })
    }
}

/// The run's context, with its state pulled; see [`agent::cobot::session`].
fn context(args: &InfraRun) -> Result<agent::cobot::session::Session, agent::cobot::Error> {
    let mut session = agent::cobot::session::open(
        args.at.instance_root(),
        &args.at.providers_dir(),
        &args.state.to_string_lossy(),
        |k| env::var(k).ok(),
    )?;
    if args.at.instance.is_some() {
        session.ctx.platform = Some(args.at.root.clone());
    }
    Ok(session)
}

/// The run's Cloudflare API, from `CLOUDFLARE_API_TOKEN` or the runner's credentials.
fn cloudflare() -> Result<agent::cobot::cloudflare::Api, agent::cobot::Error> {
    agent::cobot::cloudflare::token(
        std::env::var(agent::cobot::CREDENTIALS_VAR).ok().as_deref(),
        env::var(agent::cobot::cloudflare::TOKEN_VAR).ok(),
    )
    .map(|t| agent::cobot::cloudflare::Api::new(None, t))
    .ok_or_else(|| agent::cobot::Error::Tool {
        command: "artifacts".into(),
        message: format!(
            "set {} to a token scoped to this instance's account",
            agent::cobot::cloudflare::TOKEN_VAR
        ),
    })
}

fn git(command: GitCommand) -> Result<(), agent::cobot::Error> {
    let at: &InfraRoot = match &command {
        GitCommand::Apply(a) | GitCommand::Status(a) => a,
        GitCommand::Push(r) | GitCommand::Fetch(r) | GitCommand::Clone(r) => &r.at,
        GitCommand::Mirror(m) | GitCommand::Sync(m) => &m.at,
    };
    let root = at.instance_root().to_path_buf();
    let instance = agent::cobot::git::declared(&root, &at.root)?;
    let api = cloudflare()?;
    let client = agent::cobot::artifacts::Client::new(&api).map_err(|message| {
        agent::cobot::Error::Tool {
            command: "artifacts".into(),
            message,
        }
    })?;
    eprintln!(
        "cobot: account {}, namespace {}",
        client.account_hint(),
        instance.namespace
    );
    match command {
        GitCommand::Apply(_) => {
            for a in agent::cobot::git::apply(&client, &instance)? {
                println!(
                    "cobot: {} {:?}: {}",
                    a.name,
                    a.outcome,
                    agent::cobot::artifacts::shown_remote(&a.remote)
                );
            }
            Ok(())
        }
        GitCommand::Status(_) => {
            let mut out_of_sync = Vec::new();
            for s in agent::cobot::git::status(&client, &instance)? {
                let short = |h: &Option<String>| {
                    h.as_deref()
                        .map_or("none".to_string(), |h| h.chars().take(12).collect())
                };
                let mirror = s.mirror.as_ref().map_or("no mirror".to_string(), short);
                println!(
                    "cobot: {} {} artifacts {} mirror {mirror}",
                    s.name,
                    agent::cobot::artifacts::shown_remote(&s.remote),
                    short(&s.head)
                );
                if !s.in_sync() {
                    out_of_sync.push(s.name);
                }
            }
            if out_of_sync.is_empty() {
                Ok(())
            } else {
                Err(agent::cobot::Error::Tool {
                    command: "git status".into(),
                    message: format!(
                        "{} differ from their mirror; run `pigeon git mirror`",
                        out_of_sync.join(", ")
                    ),
                })
            }
        }
        GitCommand::Push(r) => git_run(
            &client,
            &instance,
            &r,
            "push",
            agent::cobot::artifacts::Scope::Write,
        ),
        GitCommand::Fetch(r) => git_run(
            &client,
            &instance,
            &r,
            "fetch",
            agent::cobot::artifacts::Scope::Read,
        ),
        GitCommand::Clone(r) => git_run(
            &client,
            &instance,
            &r,
            "clone",
            agent::cobot::artifacts::Scope::Read,
        ),
        GitCommand::Mirror(m) => git_mirror(&client, &instance, &m, &root, false),
        GitCommand::Sync(m) => git_mirror(&client, &instance, &m, &root, true),
    }
}

fn git_run(
    client: &agent::cobot::artifacts::Client<'_>,
    instance: &agent::cobot::git::Instance,
    r: &GitRun,
    verb: &str,
    scope: agent::cobot::artifacts::Scope,
) -> Result<(), agent::cobot::Error> {
    let mut args = vec![verb.to_string(), "{remote}".to_string()];
    args.extend(r.args.iter().cloned());
    let cwd = env::current_dir().map_err(|e| agent::cobot::Error::Io {
        path: ".".into(),
        message: e.to_string(),
    })?;
    let status = agent::cobot::git::run(client, instance, &r.repo, scope, &args, &cwd)?;
    if status.success() {
        Ok(())
    } else {
        Err(agent::cobot::Error::Tool {
            command: format!("git {verb}"),
            message: format!("exited {status}"),
        })
    }
}

/// `pigeon dev scope`: the checks a change needs; see
/// `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.
fn scope(args: &ScopeArgs) -> i32 {
    let jobs = if args.files.is_empty() {
        agent::cobot::scope::changed(&args.root, &args.base)
    } else {
        Ok(args.files.clone())
    }
    .and_then(|files| {
        let map = match &args.test_map {
            Some(p) => Some(agent::cobot::scope::TestMap::read(p)?),
            None => None,
        };
        agent::cobot::scope::jobs_with(&args.root, &files, map.as_ref())
    });
    let jobs = match jobs {
        Ok(jobs) => jobs,
        Err(e) => {
            eprintln!("cobot: {e}");
            return INVALID;
        }
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&jobs).unwrap_or_default()
        );
        return SUCCESS;
    }
    if !args.run {
        for j in &jobs {
            println!("{}: {}", j.name, j.argv.join(" "));
        }
        return SUCCESS;
    }
    match agent::cobot::scope::run(&args.root, &jobs) {
        Ok(failed) if failed.is_empty() => {
            println!("cobot: scope: {} check(s) passed", jobs.len());
            SUCCESS
        }
        Ok(failed) => {
            eprintln!("cobot: scope: failed: {}", failed.join(", "));
            VIOLATION
        }
        Err(e) => {
            eprintln!("cobot: {e}");
            INVALID
        }
    }
}

fn git_mirror(
    client: &agent::cobot::artifacts::Client<'_>,
    instance: &agent::cobot::git::Instance,
    m: &GitMirror,
    root: &std::path::Path,
    sync: bool,
) -> Result<(), agent::cobot::Error> {
    for name in &m.repos {
        match instance.repos.iter().find(|d| &d.name == name) {
            None => {
                return Err(agent::cobot::Error::UnknownUnit(format!("repos/{name}")));
            }
            Some(d) if d.mirror.is_none() => {
                return Err(agent::cobot::Error::Tool {
                    command: "git mirror".into(),
                    message: format!("repos/{name} declares no mirror"),
                });
            }
            Some(_) => {}
        }
    }
    for d in &instance.repos {
        if !m.repos.is_empty() && !m.repos.contains(&d.name) {
            continue;
        }
        let Some(to) = &d.mirror else {
            continue;
        };
        if sync {
            let done = agent::cobot::git::sync(client, instance, &d.name, to, root)?;
            let moved = done.advanced.as_ref().map_or_else(
                || "unchanged".to_string(),
                |(a, b)| format!("{}..{}", &a[..12.min(a.len())], &b[..12.min(b.len())]),
            );
            println!(
                "cobot: synced {} from {to}: main {moved}, {} tag(s) added",
                done.name, done.tags
            );
        } else {
            let done = agent::cobot::git::mirror(client, instance, &d.name, to, root)?;
            println!(
                "cobot: mirrored {} to {}: {} ref(s)",
                done.name,
                done.to,
                done.refs.len()
            );
        }
    }
    Ok(())
}

fn now() -> u64 {
    agent::cobot::session::now()
}

fn plan(args: &InfraPlan) -> Result<(), agent::cobot::Error> {
    let mut session = context(&args.run)?;
    session.ctx.adopt = args.adopt;
    let ctx = &session.ctx;
    let units = agent::cobot::select_in(
        &ctx.root,
        ctx.platform_root(),
        &ctx.providers,
        &args.run.units,
    )?;
    let plans = if args.store {
        let (stored, plans) = agent::cobot::plan_stored(ctx, &units, now())?;
        session.push()?;
        println!(
            "cobot: stored plan {} for {} unit(s); apply it with --plan {} within 24 hours",
            stored.id,
            stored.units.len(),
            stored.id
        );
        plans
    } else {
        agent::cobot::plan(ctx, &units)?
    };
    let summary = agent::cobot::summary(&plans);
    print!("{summary}");
    if let Some(path) = &args.summary {
        fs::write(path, &summary).map_err(|e| agent::cobot::Error::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
    }
    Ok(())
}

fn show(args: &InfraShow) -> Result<(), agent::cobot::Error> {
    let session = context(&args.run)?;
    for (unit, diff) in agent::cobot::show_stored(&session.ctx, &args.plan, now())? {
        println!("{unit}: {} attribute change(s)", diff.len());
        for d in diff {
            let forces = if d.replaces {
                "  (forces replacement)"
            } else {
                ""
            };
            println!("  {} {}", d.action, d.address);
            println!("    {}: {} -> {}{forces}", d.attribute, d.before, d.after);
        }
    }
    Ok(())
}

fn apply(args: &InfraApply) -> Result<(), agent::cobot::Error> {
    let mut session = context(&args.run)?;
    let applied = if let Some(id) = &args.plan {
        session.apply_stored(id, now())
    } else {
        let ctx = &session.ctx;
        agent::cobot::select_in(
            &ctx.root,
            ctx.platform_root(),
            &ctx.providers,
            &args.run.units,
        )
        .and_then(|units| agent::cobot::apply(ctx, &units))
    };
    if let Some(written) = session.push()? {
        println!("cobot: wrote {written} state object(s) to R2");
    }
    let plans = applied?;
    print!("{}", agent::cobot::summary(&plans));
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

fn secrets_pass(args: &SecretsUnit) -> Result<String, agent::cobot::Error> {
    agent::cobot::secrets_passphrase_for(
        &args.unit,
        std::env::var(agent::cobot::secrets::PASSPHRASE_VAR).ok(),
        std::env::var(agent::cobot::secrets::WORKLOAD_PASSPHRASE_VAR).ok(),
    )
}

fn secrets_path(args: &SecretsUnit) -> std::path::PathBuf {
    args.root
        .join(args.unit.trim_end_matches('/'))
        .join(agent::cobot::secrets::FILE)
}

fn secrets_set(args: &SecretsKey) -> Result<(), agent::cobot::Error> {
    let pass = secrets_pass(&args.at)?;
    let mut value = String::new();
    std::io::stdin()
        .read_to_string(&mut value)
        .map_err(|e| agent::cobot::Error::Io {
            path: "<stdin>".into(),
            message: e.to_string(),
        })?;
    let value = value.trim_end_matches(['\n', '\r']);
    let path = secrets_path(&args.at);
    agent::cobot::secrets::set(&path, &args.key, value, &pass)?;
    println!("cobot: set `{}` in {}", args.key, path.display());
    Ok(())
}

fn secrets_check(args: &SecretsUnit) -> Result<(), agent::cobot::Error> {
    let (_, values) = agent::cobot::unit_secrets(
        &args.unit,
        &secrets_path(args),
        std::env::var(agent::cobot::secrets::PASSPHRASE_VAR).ok(),
        std::env::var(agent::cobot::secrets::WORKLOAD_PASSPHRASE_VAR).ok(),
    )?;
    for key in values.keys() {
        println!("{key}: decrypts");
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(result: Result<T, agent::cobot::Error>) -> i32 {
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
    let default = ".config/releases.yaml";
    let path = args
        .config
        .clone()
        .unwrap_or_else(|| args.root.join(default));
    let config_file = path
        .strip_prefix(&args.root)
        .ok()
        .and_then(|p| p.to_str())
        .unwrap_or(default)
        .to_string();
    print_json(Config::load(&path).and_then(|config| {
        release::run(
            &args.root,
            &config,
            &config_file,
            &args.to,
            args.notes_dir.as_deref(),
            args.replay,
            args.dry_run,
        )
    }))
}
