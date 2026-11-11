//! Where traffic goes: clients, Access, the Worker, and the links between what it holds; see
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md`, the 2026-11-11
//! amendment.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

/// Backplane's sources, from the platform's root.
const SOURCES: &str = "src/pigeon-cli/crates/backplane/src";

/// What a link carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A call or an event.
    Call,
    /// A signed job, started in a Container.
    Job,
    /// A write to storage.
    Store,
    /// Git, with a minted token.
    Git,
}

/// A link, kept only while each `(file, fragment)` of its evidence is in the tree; a file's
/// fragments appear in it in order. Files are under [`SOURCES`].
#[derive(Debug, Clone, Copy)]
pub struct Row {
    /// Unique, stable across builds.
    pub id: &'static str,
    /// The part it leaves.
    pub from: &'static str,
    /// The part it reaches.
    pub to: &'static str,
    /// What the diagram writes on it.
    pub label: &'static str,
    /// What it carries.
    pub kind: Kind,
    /// Where the code does it.
    pub evidence: &'static [(&'static str, &'static str)],
}

const fn row(
    id: &'static str,
    from: &'static str,
    to: &'static str,
    label: &'static str,
    kind: Kind,
    evidence: &'static [(&'static str, &'static str)],
) -> Row {
    Row {
        id,
        from,
        to,
        label,
        kind,
        evidence,
    }
}

/// Every link the diagram may draw; nothing else adds one.
pub const LINKS: &[Row] = &[
    row(
        "worker_cr",
        "Worker",
        "ChangeRun",
        "per change",
        Kind::Call,
        &[("worker/mod.rs", "fn change_stub(")],
    ),
    row(
        "worker_inst",
        "Worker",
        "Instance",
        "per instance",
        Kind::Call,
        &[("worker/mod.rs", "durable_object(\"INSTANCE\")")],
    ),
    row(
        "worker_mr",
        "Worker",
        "MainRun",
        "main",
        Kind::Call,
        &[("worker/mod.rs", "fn main_stub(")],
    ),
    row(
        "cr_inst",
        "ChangeRun",
        "Instance",
        "events · queue",
        Kind::Call,
        &[
            ("worker/change.rs", "super::events::"),
            ("worker/change.rs", "super::merges::"),
        ],
    ),
    row(
        "mr_inst",
        "MainRun",
        "Instance",
        "events · push",
        Kind::Call,
        &[("worker/main_run.rs", "super::events::")],
    ),
    row(
        "cr_cr",
        "ChangeRun",
        "ChangeRun",
        "others: behind",
        Kind::Call,
        &[
            ("worker/merges.rs", "Event::Behind"),
            ("worker/merges.rs", "to_change("),
        ],
    ),
    row(
        "cr_runner",
        "ChangeRun",
        "Runner",
        "bound · merge · restack",
        Kind::Job,
        &[
            ("worker/change.rs", "job_stub"),
            ("control.rs", "fn class_of("),
        ],
    ),
    row(
        "cr_checker",
        "ChangeRun",
        "Checker",
        "code checks",
        Kind::Job,
        &[
            ("worker/change.rs", "job_stub"),
            ("control.rs", "Class::Checker"),
        ],
    ),
    row(
        "mr_checker",
        "MainRun",
        "Checker",
        "deploy",
        Kind::Job,
        &[("worker/main_run.rs", "class_of(control::DEPLOY)")],
    ),
    row(
        "mr_runner",
        "MainRun",
        "Runner",
        "mirror",
        Kind::Job,
        &[("worker/main_run.rs", "class_of(control::MIRROR)")],
    ),
    row(
        "cr_d1",
        "ChangeRun",
        "D1",
        "changes · jobs",
        Kind::Store,
        &[("worker/change.rs", "platform::d1(")],
    ),
    row(
        "cr_r2",
        "ChangeRun",
        "R2",
        "log tails",
        Kind::Store,
        &[("worker/change.rs", "bucket(\"CI_LOGS\")")],
    ),
    row(
        "mr_r2",
        "MainRun",
        "R2",
        "deploy logs",
        Kind::Store,
        &[("worker/main_run.rs", "bucket(\"CI_LOGS\")")],
    ),
    row(
        "checker_art",
        "Checker",
        "Artifacts",
        "fetch · read token",
        Kind::Git,
        &[("worker/platform.rs", "fn credentials")],
    ),
    row(
        "runner_art",
        "Runner",
        "Artifacts",
        "push · write token",
        Kind::Git,
        &[("worker/change.rs", "credentials(&self.env, write")],
    ),
    row(
        "cr_actions",
        "ChangeRun",
        "Actions",
        "dispatch ci.yml · read runs",
        Kind::Call,
        &[
            ("worker/change.rs", "github::dispatch("),
            ("worker/change.rs", "github::run("),
        ],
    ),
    row(
        "runner_mirror",
        "Runner",
        "Mirror",
        "main + tags, fast-forward",
        Kind::Git,
        &[
            ("worker/main_run.rs", "class_of(control::MIRROR)"),
            ("worker/github.rs", "pub fn mirror("),
        ],
    ),
    row(
        "actions_checker",
        "Actions",
        "Checker",
        "waiting: GitHub, retried; ci_rerun on: cloudflare runs them on Checker",
        Kind::Job,
        &[
            ("actions.rs", "waiting: GitHub"),
            ("worker/change.rs", "\"cloudflare\""),
        ],
    ),
];

/// A row of a card: a name and what it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    /// The name, such as a path.
    pub k: String,
    /// What it says; empty for none.
    pub v: String,
}

fn item(k: impl Into<String>, v: impl Into<String>) -> Item {
    Item {
        k: k.into(),
        v: v.into(),
    }
}

/// One kind of client, and the routes it uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Client {
    /// `mcp`, `git` or `owner`.
    pub id: String,
    /// What the diagram shows.
    pub title: String,
    /// Its routes.
    pub items: Vec<Item>,
}

/// Access, in front of the Worker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Access {
    /// The hostnames it protects.
    pub hosts: Vec<String>,
    /// Its service tokens' names.
    pub tokens: Vec<String>,
}

/// A Container class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Container {
    /// Its class.
    pub class: String,
    /// Its instance type.
    pub instance_type: String,
    /// The jobs it runs; none means every job the others don't.
    pub jobs: Vec<String>,
}

/// The control plane's Worker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Worker {
    /// Its script name.
    pub name: String,
    /// How many routes it serves.
    pub routes: usize,
    /// How many queues it consumes; their names are an instance's own.
    pub queues: usize,
    /// Its crons.
    pub crons: Vec<String>,
    /// How many secrets it holds; never their names' values.
    pub secrets: usize,
    /// Its Durable Object classes that are not Containers.
    pub objects: Vec<String>,
    /// Its Containers.
    pub containers: Vec<Container>,
    /// Its D1 bindings.
    pub d1: Vec<String>,
    /// Its R2 bindings.
    pub r2: Vec<String>,
}

/// A box of the connection diagram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Part {
    /// Unique; a link's ends.
    pub id: String,
    /// `object`, `container`, `database`, `bucket`, `git` or `github`.
    pub kind: String,
    /// What it does.
    pub role: String,
    /// Whether there is one per change.
    pub many: bool,
    /// The file it was derived from.
    pub source: String,
}

/// A kept link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// Its row's id.
    pub id: String,
    /// The part it leaves.
    pub from: String,
    /// The part it reaches.
    pub to: String,
    /// What the diagram writes on it.
    pub label: String,
    /// What it carries.
    pub kind: Kind,
    /// Its evidence's files.
    pub source: String,
}

/// Where traffic goes, top to bottom.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Network {
    /// Clients, from the Worker's routes.
    pub clients: Vec<Client>,
    /// Access, from the platform unit.
    pub access: Access,
    /// The Worker, from its spec.
    pub worker: Worker,
    /// The GitHub repository it mirrors to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// The connection diagram's boxes.
    pub parts: Vec<Part>,
    /// The kept links.
    pub links: Vec<Link>,
    /// Where the parts sit and how the links run.
    pub diagram: super::place::Diagram,
}

fn read(platform: &Path, file: &str) -> Option<String> {
    std::fs::read_to_string(platform.join(SOURCES).join(file)).ok()
}

/// Whether `row`'s evidence is in the tree at `platform`.
#[must_use]
pub fn evidenced(platform: &Path, row: &Row) -> bool {
    let mut files: BTreeMap<&str, (String, usize)> = BTreeMap::new();
    row.evidence.iter().all(|(file, fragment)| {
        if !files.contains_key(file) {
            let Some(text) = read(platform, file) else {
                return false;
            };
            files.insert(file, (text, 0));
        }
        let Some((text, at)) = files.get_mut(file) else {
            return false;
        };
        match text[*at..].find(fragment) {
            Some(i) => {
                *at += i + fragment.len();
                true
            }
            None => false,
        }
    })
}

/// The network of the instance at `root`, served by the platform at `platform`; none when the
/// platform has no backplane Worker.
#[must_use]
pub fn network(root: &Path, platform: &Path) -> Option<Network> {
    let found = cobot::workers::discover(platform)
        .ok()?
        .into_iter()
        .find(|f| f.spec.entry.krate.ends_with("crates/backplane"))?;
    let kind = found.spec.deployment.clone();
    let (manifest, _) = cobot::manifest::load(root, platform)?;
    let stacks: Vec<&cobot::manifest::Entry> = manifest
        .units
        .values()
        .filter(|e| Some(&e.kind) == kind.as_ref())
        .collect();
    let stack = manifest
        .units
        .iter()
        .find(|(_, e)| Some(&e.kind) == kind.as_ref())
        .map(|(id, _)| id.clone());
    let spec = match &stack {
        Some(unit) => cobot::workers::bind(root, &found, unit).ok()?.spec,
        None => found.spec,
    };
    let access = access(&stacks);
    let routes = routes(platform);
    let worker = worker(platform, &spec, &routes);
    let github = spec
        .bindings
        .vars
        .get("MIRROR")
        .and_then(|m| m.strip_prefix("https://github.com/"))
        .map(String::from);
    let repos: Vec<String> = manifest
        .units
        .iter()
        .filter(|(_, e)| e.kind == "cloudflare/git-repo")
        .map(|(id, _)| id.rsplit('/').next().unwrap_or(id).to_string())
        .collect();
    let parts = parts(platform, &worker, &repos, github.as_deref());
    let known = |id: &str| id == "Worker" || parts.iter().any(|p| p.id == id);
    let links: Vec<Link> = LINKS
        .iter()
        .filter(|r| known(r.from) && known(r.to) && evidenced(platform, r))
        .map(|r| Link {
            id: r.id.into(),
            from: r.from.into(),
            to: r.to.into(),
            label: r.label.into(),
            kind: r.kind,
            source: {
                let mut files: Vec<&str> = r.evidence.iter().map(|(f, _)| *f).collect();
                files.dedup();
                files.join(" ")
            },
        })
        .collect();
    let diagram = super::place::place(&parts, &links);
    Some(Network {
        clients: clients(&routes),
        access,
        worker,
        github,
        parts,
        links,
        diagram,
    })
}

/// Each surface's routes, by its file's stem, as `METHOD path`; a prefix ends in `…`.
fn routes(platform: &Path) -> Vec<(String, Vec<String>)> {
    let dir = platform.join(SOURCES).join("worker");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(start) = text.find("ROUTES: &[") else {
            continue;
        };
        let body = &text[start..];
        let body = &body[..body.find("];").unwrap_or(body.len())];
        let mut found = Vec::new();
        let mut method = None;
        let mut rest = body;
        while let Some(i) = rest.find("::") {
            let before = &rest[..i];
            let word = before
                .rsplit(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap_or("");
            let after = &rest[i + 2..];
            let name: String = after.chars().take_while(|c| c.is_alphanumeric()).collect();
            rest = after;
            match (word, name.as_str()) {
                ("Method", m) => method = Some(m.to_uppercase()),
                ("Path", "Exact" | "Prefix") => {
                    let Some(q) = rest.find('"') else { break };
                    let tail = &rest[q + 1..];
                    let Some(end) = tail.find('"') else { break };
                    let p = &tail[..end];
                    let p = if name == "Prefix" {
                        format!("{p}…")
                    } else {
                        p.to_string()
                    };
                    found.push(match method.take() {
                        Some(m) => format!("{m} {p}"),
                        None => p,
                    });
                    rest = &tail[end + 1..];
                }
                _ => {}
            }
        }
        if !found.is_empty() {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((stem, found));
        }
    }
    out
}

/// The clients: MCP from the API surface, git from the git door, and the owner's pages from the
/// rest, without their scripts and sub-paths.
fn clients(routes: &[(String, Vec<String>)]) -> Vec<Client> {
    let tools = crate::mcp::tools();
    let tools = tools["tools"].as_array().cloned().unwrap_or_default();
    let read_only = tools
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == serde_json::Value::Bool(true))
        .count();
    let mut mcp = Vec::new();
    let mut git = Vec::new();
    let mut owner: Vec<String> = Vec::new();
    for (stem, paths) in routes {
        for p in paths {
            match stem.as_str() {
                "api" if p.ends_with(" /mcp") => {
                    mcp.push(item(
                        p.clone(),
                        format!("{} tools · {read_only} read-only", tools.len()),
                    ));
                }
                "api" => mcp.push(item(p.clone(), "")),
                "gitdoor" => git.push(item(p.clone(), "fetch, and push to a change's branch")),
                _ => owner.push(p.clone()),
            }
        }
    }
    let path = |p: &str| {
        p.rsplit(' ')
            .next()
            .unwrap_or(p)
            .trim_end_matches('…')
            .to_string()
    };
    let pages: Vec<Item> = owner
        .iter()
        .filter(|p| !path(p).contains('.'))
        .filter(|p| {
            let me = path(p);
            !owner.iter().any(|o| {
                let other = path(o);
                other != me && me.starts_with(&format!("{}/", other.trim_end_matches('/')))
            })
        })
        .map(|p| item(p.clone(), ""))
        .collect();
    [
        ("mcp", "MCP clients", mcp),
        ("git", "git", git),
        ("owner", "Owner's browser", pages),
    ]
    .into_iter()
    .filter(|(_, _, items)| !items.is_empty())
    .map(|(id, title, items)| Client {
        id: id.into(),
        title: title.into(),
        items,
    })
    .collect()
}

/// The jobs `class_of` sends to `Runner`, by their constants' names.
fn runner_jobs(platform: &Path) -> Vec<String> {
    let Some(text) = read(platform, "control.rs") else {
        return Vec::new();
    };
    let Some(start) = text.find("pub fn class_of(") else {
        return Vec::new();
    };
    let body = &text[start..];
    let body = &body[..body.find("Class::Runner").unwrap_or(0)];
    body.split("job == ")
        .skip(1)
        .filter_map(|s| {
            let name: String = s
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            name.rsplit("::").next().map(str::to_lowercase)
        })
        .collect()
}

/// The doc comment above `pub struct <class>` in the Worker's sources: its first clause, and
/// whether its name carries a change's number.
fn doc(platform: &Path, class: &str) -> Option<(String, bool, String)> {
    let dir = platform.join(SOURCES).join("worker");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let Some(at) = lines
            .iter()
            .position(|l| l.trim() == format!("pub struct {class} {{"))
        else {
            continue;
        };
        let mut docs: Vec<&str> = lines[..at]
            .iter()
            .rev()
            .skip_while(|l| l.trim_start().starts_with("#["))
            .take_while(|l| l.trim_start().starts_with("///"))
            .map(|l| l.trim_start().trim_start_matches("///").trim())
            .collect();
        docs.reverse();
        let text = docs.join(" ");
        let first = text
            .split([';', '.'])
            .next()
            .unwrap_or("")
            .split(" (")
            .next()
            .unwrap_or("")
            .replace('`', "");
        let file = format!(
            "{SOURCES}/worker/{}",
            path.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        return Some((first, text.contains("<number>"), file));
    }
    None
}

fn parts(platform: &Path, w: &Worker, repos: &[String], github: Option<&str>) -> Vec<Part> {
    let part = |id: &str, kind: &str, role: String, many: bool, source: &str| Part {
        id: id.into(),
        kind: kind.into(),
        role,
        many,
        source: source.into(),
    };
    let spec = "infra/pigeon/workers/control/worker.yaml";
    let mut out = Vec::new();
    for class in &w.objects {
        let (role, many, source) = doc(platform, class).unwrap_or_default();
        out.push(part(class, "object", role, many, &source));
    }
    for c in &w.containers {
        let jobs = if c.jobs.is_empty() {
            "every other job".to_string()
        } else {
            c.jobs.join(", ")
        };
        out.push(part(
            &c.class,
            "container",
            format!("{} · {jobs}", c.instance_type),
            false,
            spec,
        ));
    }
    if !w.d1.is_empty() {
        out.push(part("D1", "database", w.d1.join(" · "), false, spec));
    }
    if !w.r2.is_empty() {
        out.push(part("R2", "bucket", w.r2.join(" · "), false, spec));
    }
    if !repos.is_empty() {
        out.push(part(
            "Artifacts",
            "git",
            repos.join(" · "),
            false,
            "config.yaml",
        ));
    }
    if let Some(repo) = github {
        out.push(part(
            "Actions",
            "github",
            format!("{repo}: main's ci.yml"),
            false,
            spec,
        ));
        out.push(part(
            "Mirror",
            "github",
            format!("{repo}: main and tags"),
            false,
            spec,
        ));
    }
    out
}

/// Access, from the platform units' domains and service tokens.
fn access(stacks: &[&cobot::manifest::Entry]) -> Access {
    let mut access = Access::default();
    for e in stacks {
        let strings = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        };
        access.hosts.extend(strings(&e.settings["domains"]));
        for t in e.settings["service_tokens"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(n) = t["name"].as_str() {
                access.tokens.push(n.to_string());
            }
        }
    }
    access.hosts.dedup();
    access.tokens.dedup();
    access
}

/// The Worker, from its bound spec.
fn worker(
    platform: &Path,
    spec: &cobot::workers::Spec,
    routes: &[(String, Vec<String>)],
) -> Worker {
    let containers: Vec<String> = spec.containers.iter().map(|c| c.class.clone()).collect();
    let runner = runner_jobs(platform);
    Worker {
        name: spec.name.clone(),
        routes: routes.iter().map(|(_, r)| r.len()).sum(),
        queues: spec.bindings.queues.consumers.len(),
        crons: spec.crons.clone(),
        secrets: spec.secrets.len(),
        objects: spec
            .durable_objects
            .classes
            .iter()
            .filter(|c| !containers.contains(c))
            .cloned()
            .collect(),
        containers: spec
            .containers
            .iter()
            .map(|c| Container {
                class: c.class.clone(),
                instance_type: c.instance_type.clone(),
                jobs: if c.class == "Runner" {
                    runner.clone()
                } else {
                    Vec::new()
                },
            })
            .collect(),
        d1: spec.bindings.d1.keys().cloned().collect(),
        r2: spec.bindings.r2.keys().cloned().collect(),
    }
}
