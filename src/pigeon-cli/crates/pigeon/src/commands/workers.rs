//! `pigeon cobot workers`; see `.claude/decisions/2026-11-05-development-cobot-workers-area.md`.

use std::collections::BTreeMap;
use std::env;
use std::io::Read;
use std::path::Path;

use cobot::session::Session;
use cobot::workers::{self, Found, Tools};
use serde_json::{Map, Value};

use crate::cli::{WorkersCommand, WorkersSecrets, WorkersState};

/// Runs a `pigeon cobot workers` command.
///
/// # Errors
///
/// As each command's engine function.
pub fn run(command: WorkersCommand) -> Result<(), cobot::Error> {
    match command {
        WorkersCommand::Validate(at) => {
            let found = workers::select(&at.root, &at.names)?;
            let problems: Vec<_> = found
                .iter()
                .flat_map(|f| workers::validate(&at.root, f))
                .collect();
            if !problems.is_empty() {
                return Err(cobot::Error::Invalid(problems));
            }
            println!("cobot: {} Worker spec(s) valid", found.len());
            Ok(())
        }
        WorkersCommand::Permissions(at) => {
            let specs: Vec<_> = workers::select(&at.root, &at.names)?
                .into_iter()
                .map(|f| f.spec)
                .collect();
            for p in workers::permissions(&specs) {
                println!("{p}");
            }
            Ok(())
        }
        WorkersCommand::Render(s) => {
            for (f, rendered) in rendered(&s)?.1 {
                let dir = workers::write(&s.at.root, &f, &rendered, "render")?;
                println!("cobot: wrote {}", dir.join("wrangler.jsonc").display());
            }
            Ok(())
        }
        WorkersCommand::Plan(s) => plan(&s),
        WorkersCommand::Deploy(d) => deploy(&d.run, &d.plan),
        WorkersCommand::Status(at) => {
            let tools = tools()?;
            for f in workers::select(&at.root, &at.names)? {
                print!("{}", workers::status(&f, &tools, &at.root)?);
            }
            Ok(())
        }
        WorkersCommand::Secrets(WorkersSecrets::Put(p)) => {
            let f = one(&p.worker.root, &p.worker.name)?;
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .map_err(|e| cobot::Error::Tool {
                    command: "stdin".into(),
                    message: e.to_string(),
                })?;
            let value = value.trim_end_matches(['\r', '\n']);
            workers::put_secret(&f, &p.key, value, &tools()?, &p.worker.root)?;
            println!("cobot: set {} on {}", p.key, f.spec.name);
            Ok(())
        }
        WorkersCommand::Teardown(o) => {
            let f = one(&o.root, &o.name)?;
            for d in workers::teardown(&f, &tools()?, &o.root)? {
                println!("cobot: {d}");
            }
            Ok(())
        }
    }
}

fn one(root: &Path, name: &str) -> Result<Found, cobot::Error> {
    workers::select(root, &[name.to_string()])?
        .into_iter()
        .next()
        .ok_or_else(|| cobot::Error::UnknownUnit(format!("workers/{name}")))
}

/// wrangler and docker, with the run's account; the token and account are scrubbed from output.
fn tools() -> Result<Tools, cobot::Error> {
    let token = cobot::cloudflare::token(
        env::var(cobot::CREDENTIALS_VAR).ok().as_deref(),
        env::var(cobot::cloudflare::TOKEN_VAR).ok(),
    )
    .ok_or_else(|| cobot::Error::Tool {
        command: "workers".into(),
        message: format!(
            "set {} to a token with `pigeon cobot workers permissions`",
            cobot::cloudflare::TOKEN_VAR
        ),
    })?;
    let api = cobot::cloudflare::Api::new(None, token.clone());
    let account = api.account().map_err(|message| cobot::Error::Tool {
        command: "workers".into(),
        message,
    })?;
    eprintln!(
        "cobot: account …{}",
        &account[account.len().saturating_sub(4)..]
    );
    let mut t = Tools::from(|k| env::var(k).ok());
    t.env = vec![
        ("CLOUDFLARE_ACCOUNT_ID".into(), account),
        (cobot::cloudflare::TOKEN_VAR.into(), token.clone()),
    ];
    t.secrets.push(token);
    Ok(t)
}

/// Each selected Worker and its configuration, references resolved from the state, and the
/// state's session.
fn rendered(s: &WorkersState) -> Result<(Session, Vec<(Found, Value)>), cobot::Error> {
    let found = workers::select(&s.at.root, &s.at.names)?;
    let problems: Vec<_> = found
        .iter()
        .flat_map(|f| workers::validate(&s.at.root, f))
        .collect();
    if !problems.is_empty() {
        return Err(cobot::Error::Invalid(problems));
    }
    let providers = s
        .providers
        .clone()
        .unwrap_or_else(|| s.at.root.join(".config"));
    let session = cobot::session::open(&s.at.root, &providers, &s.state.to_string_lossy(), |k| {
        env::var(k).ok()
    })?;
    let mut outputs: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    for f in &found {
        for unit in workers::referenced_units(&f.spec) {
            if let std::collections::btree_map::Entry::Vacant(e) = outputs.entry(unit) {
                let o = cobot::outputs(&session.ctx, e.key())?;
                e.insert(o);
            }
        }
    }
    let all = found
        .into_iter()
        .map(|f| match workers::render(&f, &outputs) {
            Ok(v) => Ok((f, v)),
            Err(problems) => Err(cobot::Error::Invalid(
                problems
                    .into_iter()
                    .map(|message| cobot::Problem {
                        path: format!("{}/{}", f.dir, workers::SPEC),
                        message,
                    })
                    .collect(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((session, all))
}

fn plan(s: &WorkersState) -> Result<(), cobot::Error> {
    let (mut session, all) = rendered(s)?;
    let map: BTreeMap<String, Value> = all
        .iter()
        .map(|(f, v)| (f.spec.name.clone(), v.clone()))
        .collect();
    let stored = workers::store(&session.ctx.state, &map, cobot::session::now())?;
    session.push()?;
    for (f, v) in &all {
        workers::write(&s.at.root, f, v, &stored.id)?;
        println!("cobot: {} planned", f.spec.name);
    }
    println!(
        "cobot: stored Worker plan {}; deploy it with --plan {} within 24 hours",
        stored.id, stored.id
    );
    Ok(())
}

fn deploy(s: &WorkersState, id: &str) -> Result<(), cobot::Error> {
    let (session, all) = rendered(s)?;
    let stored = workers::load(&session.ctx.state, id, cobot::session::now())?;
    let tools = tools()?;
    for (f, v) in &all {
        let Some(want) = stored.workers.get(&f.spec.name) else {
            continue;
        };
        if &workers::digest(v) != want {
            return Err(cobot::Error::Plan(format!(
                "{} changed since plan {id}; plan again",
                f.spec.name
            )));
        }
    }
    for (f, v) in &all {
        if !stored.workers.contains_key(&f.spec.name) {
            continue;
        }
        let dir = workers::write(&s.at.root, f, v, id)?;
        for step in workers::deploy(&s.at.root, f, &dir, id, &tools)? {
            println!("cobot: {step}");
        }
        let missing = workers::missing_secrets(f, &tools, &dir)?;
        if !missing.is_empty() {
            println!(
                "cobot: {} still needs secret(s) {}; set each with `pigeon cobot workers secrets put {} KEY`",
                f.spec.name,
                missing.join(", "),
                f.spec.name
            );
        }
    }
    Ok(())
}
