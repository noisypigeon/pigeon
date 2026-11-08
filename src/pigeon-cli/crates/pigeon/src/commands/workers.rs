//! `pigeon cobot workers`; see `.claude/decisions/2026-11-05-development-cobot-workers-area.md`.

use std::collections::BTreeMap;
use std::env;
use std::io::Read;
use std::path::Path;

use cobot::session::Session;
use cobot::workers::{self, Found, Tools};
use serde_json::{Map, Value};

use crate::cli::{ImagesPush, WorkersCommand, WorkersImages, WorkersSecrets, WorkersState};

/// Runs a `pigeon cobot workers` command.
///
/// # Errors
///
/// As each command's engine function.
pub fn run(command: WorkersCommand) -> Result<(), cobot::Error> {
    match command {
        WorkersCommand::Validate(at) => {
            let found = workers::select_bound(&at.root, &at.names, at.deployment.as_deref())?;
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
            let specs: Vec<_> =
                workers::select_bound(&at.root, &at.names, at.deployment.as_deref())?
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
            for f in workers::select_bound(&at.root, &at.names, at.deployment.as_deref())? {
                print!("{}", workers::status(&f, &tools, &at.root)?);
            }
            Ok(())
        }
        WorkersCommand::Secrets(WorkersSecrets::Put(p)) => {
            let f = one(
                &p.worker.root,
                &p.worker.name,
                p.worker.deployment.as_deref(),
            )?;
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .map_err(|e| cobot::Error::Tool {
                    command: "stdin".into(),
                    message: e.to_string(),
                })?;
            let value = value.trim_end_matches(['\r', '\n']);
            // A pipe whose producer failed sends nothing; a secret is never set empty.
            if value.is_empty() {
                return Err(cobot::Error::Tool {
                    command: "secrets put".into(),
                    message: format!("no value on stdin; {} is unchanged", p.key),
                });
            }
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
        WorkersCommand::Images(WorkersImages::Push(p)) => images_push(&p),
        WorkersCommand::Teardown(o) => {
            let f = one(&o.root, &o.name, o.deployment.as_deref())?;
            for d in workers::teardown(&f, &tools()?, &o.root)? {
                println!("cobot: {d}");
            }
            Ok(())
        }
    }
}

fn one(root: &Path, name: &str, deployment: Option<&str>) -> Result<Found, cobot::Error> {
    workers::select_bound(root, &[name.to_string()], deployment)?
        .into_iter()
        .next()
        .ok_or_else(|| cobot::Error::UnknownUnit(format!("workers/{name}")))
}

/// wrangler and docker, with the run's account; the token and account are scrubbed from output.
fn tools() -> Result<Tools, cobot::Error> {
    Ok(account_tools()?.0)
}

/// As [`tools`], with the API and the account they run against.
fn account_tools() -> Result<(Tools, cobot::cloudflare::Api, String), cobot::Error> {
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
        ("CLOUDFLARE_ACCOUNT_ID".into(), account.clone()),
        (cobot::cloudflare::TOKEN_VAR.into(), token.clone()),
    ];
    t.secrets.push(token);
    Ok((t, api, account))
}

/// Each selected Worker, checked, the state's session, and the outputs their references name.
fn state_outputs(
    s: &WorkersState,
) -> Result<(Session, Vec<Found>, cobot::ship::Outputs), cobot::Error> {
    let found = workers::select_bound(&s.at.root, &s.at.names, s.at.deployment.as_deref())?;
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

/// `git <args>` in `root`; its trimmed output, or none when it fails.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Builds and pushes each selected Worker's images, tagged with the checkout's commit, which
/// must be clean; prints the references as JSON. See
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §1.
fn images_push(p: &ImagesPush) -> Result<(), cobot::Error> {
    let root = &p.at.root;
    let refused = |message: String| cobot::Error::Tool {
        command: "images push".into(),
        message,
    };
    let source = git(root, &["rev-parse", "HEAD"])
        .ok_or_else(|| refused("the root is not a git checkout".into()))?;
    let dirty = git(root, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
    if !dirty.is_empty() {
        return Err(refused(format!(
            "the checkout has uncommitted changes, so the images wouldn't match commit {}:\n{dirty}",
            &source[..12]
        )));
    }
    let mains = git(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/remotes/*/main"],
    )
    .unwrap_or_default();
    let on_main = mains
        .lines()
        .chain(std::iter::once("FETCH_HEAD"))
        .any(|r| git(root, &["merge-base", "--is-ancestor", "HEAD", r]).is_some());
    if !on_main {
        eprintln!(
            "cobot: warning: {} is on no fetched main; a deploy can't diff from a commit outside main's history, so push from main after `pigeon cobot git fetch <repo> main`",
            &source[..12]
        );
    }
    let (tools, api, account) = account_tools()?;
    let tag = source.chars().take(12).collect::<String>();
    let mut images = cobot::ship::Images {
        source,
        images: BTreeMap::new(),
    };
    for f in workers::select_bound(root, &p.at.names, p.at.deployment.as_deref())? {
        let pushed =
            workers::push_images(root, &f, &tag, &account, &api, p.expiration_minutes, &tools)?;
        for (k, v) in pushed {
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
