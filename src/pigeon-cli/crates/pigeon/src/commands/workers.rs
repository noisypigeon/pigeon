//! `pigeon cobot workers`; see `.claude/decisions/2026-11-05-development-cobot-workers-area.md`.

use std::collections::BTreeMap;
use std::env;
use std::io::Read;
use std::path::Path;

use cobot::session::Session;
use cobot::workers::{self, Found, Tools};
use serde_json::{Map, Value};

use crate::cli::{WorkersAt, WorkersCommand, WorkersImages, WorkersSecrets, WorkersState};

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
        WorkersCommand::Outputs(s) => {
            let (_, found, outputs) = state_outputs(&s)?;
            let specs: Vec<_> = found.into_iter().map(|f| f.spec).collect();
            let snapshot = cobot::ship::snapshot(&specs, &outputs);
            println!("{}", serde_json::to_string(&snapshot).unwrap_or_default());
            eprintln!(
                "cobot: {} unit(s) of outputs; pipe this to `pigeon cobot workers secrets put --root . <worker> {}`",
                snapshot.len(),
                cobot::ship::OUTPUTS_VAR
            );
            Ok(())
        }
        WorkersCommand::Images(WorkersImages::Push(at)) => images_push(&at),
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

/// Each selected Worker, checked, the state's session, and the outputs their references name.
fn state_outputs(
    s: &WorkersState,
) -> Result<(Session, Vec<Found>, cobot::ship::Outputs), cobot::Error> {
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
    Ok((session, found, outputs))
}

/// Each selected Worker and its configuration, references resolved from the state, and the
/// state's session.
fn rendered(s: &WorkersState) -> Result<(Session, Vec<(Found, Value)>), cobot::Error> {
    let (session, found, outputs) = state_outputs(s)?;
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

/// Builds and pushes each selected Worker's images, tagged with the checkout's commit; prints
/// the references as JSON.
fn images_push(at: &WorkersAt) -> Result<(), cobot::Error> {
    let source = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&at.root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok_or_else(|| cobot::Error::Tool {
            command: "git rev-parse HEAD".into(),
            message: "the root is not a git checkout".into(),
        })?;
    let tools = tools()?;
    let account = tools
        .env
        .iter()
        .find(|(k, _)| k == "CLOUDFLARE_ACCOUNT_ID")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let tag = source.chars().take(12).collect::<String>();
    let mut images = cobot::ship::Images {
        source,
        images: BTreeMap::new(),
    };
    for f in workers::select(&at.root, &at.names)? {
        for (k, v) in workers::push_images(&at.root, &f, &tag, &account, &tools)? {
            eprintln!("cobot: pushed {k}");
            images.images.insert(k, v);
        }
    }
    println!("{}", serde_json::to_string(&images).unwrap_or_default());
    eprintln!(
        "cobot: pipe this to `pigeon cobot workers secrets put --root . <worker> {}`",
        cobot::ship::IMAGES_VAR
    );
    Ok(())
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
