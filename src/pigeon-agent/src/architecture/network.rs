//! Where traffic goes: clients, Access, the Worker, and the links between what it holds; see
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md`, the 2026-11-11
//! amendment.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

/// The control plane's sources, from the platform's root: agent's, which holds the Worker, then
/// core's.
const SOURCES: [&str; 2] = ["src/pigeon-agent/src", "src/pigeon-core/src"];

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
        "worker_docs",
        "Worker",
        "R2",
        "docs site, behind a passkey",
        Kind::Store,
        &[("worker/docs.rs", "env.bucket(BINDING)")],
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
        "worker_store",
        "Worker",
        "Secrets Store",
        "granter: mints scoped tokens",
        Kind::Call,
        &[("worker/minting.rs", ".secret_store(GRANTER)")],
    ),
    row(
        "worker_journal_seal",
        "Worker",
        "Secrets Store",
        "journal-seal: seals secret-bearing journal entries",
        Kind::Call,
        &[("worker/mod.rs", ".secret_store(JOURNAL_SEAL)")],
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

/// A service outside Cloudflare the Worker calls; see
/// `.claude/decisions/2026-11-15-runtime-architecture-coverage-agents-deploys.md` §1.
#[derive(Debug, Clone, Copy)]
pub struct Service {
    /// Its part's id, and what the diagram shows.
    pub id: &'static str,
    /// The hosts the code calls it at; evidence only, never shown.
    pub hosts: &'static [&'static str],
    /// What the Worker uses it for.
    pub label: &'static str,
    /// For a service with no fixed host: where the code calls it.
    pub evidence: &'static [(&'static str, &'static str)],
    /// Whether it is drawn as another part already, such as GitHub's.
    pub drawn: bool,
}

/// Every outbound service; a row is kept while one of its hosts is in a non-test string
/// literal of [`SOURCES`], or, with no host, while its evidence is.
pub const SERVICES: &[Service] = &[
    Service {
        id: "Cloudflare API",
        hosts: &["api.cloudflare.com"],
        label: "mints tokens · insights GraphQL",
        evidence: &[],
        drawn: false,
    },
    Service {
        id: "APNs",
        hosts: &["api.push.apple.com"],
        label: "pushes to the owner's iPhone",
        evidence: &[],
        drawn: false,
    },
    Service {
        id: "Web Push",
        hosts: &[],
        label: "approvals · digest, to the owner's browsers",
        evidence: &[("worker/push.rs", "Vapid::from_secret(")],
        drawn: false,
    },
    Service {
        id: "App Store Connect",
        hosts: &["api.appstoreconnect.apple.com", "www.apple.com"],
        label: "ios.release: TestFlight uploads, signing",
        evidence: &[],
        drawn: false,
    },
    Service {
        id: "Bitrise",
        hosts: &["api.bitrise.io"],
        label: "macOS builds, failover",
        evidence: &[],
        drawn: false,
    },
    Service {
        id: "Buildkite",
        hosts: &["api.buildkite.com", "agent.buildkite.com"],
        label: "macOS builds, failover · signing pipeline",
        evidence: &[],
        drawn: false,
    },
    Service {
        id: "AI Gateway",
        hosts: &["gateway.ai.cloudflare.com"],
        label: "capacity pools' model requests, to model providers via pigeon-models",
        evidence: &[],
        drawn: false,
    },
    Service {
        id: "GitHub",
        hosts: &["api.github.com", "github.com"],
        label: "Actions and the mirror",
        evidence: &[],
        drawn: true,
    },
];

/// Whether `host` is one of the platform's own, a Durable Object's or a test's.
#[must_use]
pub fn internal(host: &str) -> bool {
    !host.contains('.')
        || host == "pigeon.dev"
        || host.ends_with(".pigeon.dev")
        || host.ends_with(".example")
        || host.ends_with(".invalid")
        || host == "example.com"
}

/// The hosts in `https://` string literals of `text`, before its tests.
#[must_use]
pub fn hosts_in(text: &str) -> Vec<String> {
    let text = text.split("#[cfg(test)]").next().unwrap_or("");
    let mut out: Vec<String> = text
        .split("\"https://")
        .skip(1)
        .map(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect::<String>()
        })
        .filter(|h| !h.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every Rust file under [`SOURCES`] the Worker's code may call out from: all but the engines',
/// which run in Containers, and this module's.
#[must_use]
pub fn outbound_files(platform: &Path) -> Vec<std::path::PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            if p.is_dir() {
                if !matches!(
                    name.as_deref(),
                    Some(
                        "infra"
                            | "workers"
                            | "sites"
                            | "git"
                            | "release"
                            | "checks"
                            | "containers"
                            | "architecture"
                            | "tests"
                    )
                ) {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs")
                && !name
                    .as_deref()
                    .is_some_and(|n| n == "tests.rs" || n.ends_with("_tests.rs"))
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for s in SOURCES {
        walk(&platform.join(s), &mut out);
    }
    out
}

/// The outbound services kept, each with the files it was found in.
fn services(platform: &Path) -> Vec<(Service, String)> {
    let roots: Vec<_> = SOURCES.iter().map(|s| platform.join(s)).collect();
    let found: Vec<(String, Vec<String>)> = outbound_files(platform)
        .iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(p).ok()?;
            let rel = roots
                .iter()
                .find_map(|r| p.strip_prefix(r).ok())
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_default();
            Some((rel, hosts_in(&text)))
        })
        .collect();
    SERVICES
        .iter()
        .filter(|s| !s.drawn)
        .filter_map(|s| {
            let files: Vec<String> = if s.hosts.is_empty() {
                let row = Row {
                    id: s.id,
                    from: "Worker",
                    to: s.id,
                    label: s.label,
                    kind: Kind::Call,
                    evidence: s.evidence,
                };
                if !evidenced(platform, &row) {
                    return None;
                }
                s.evidence.iter().map(|(f, _)| (*f).to_string()).collect()
            } else {
                found
                    .iter()
                    .filter(|(_, hs)| hs.iter().any(|h| s.hosts.contains(&h.as_str())))
                    .map(|(f, _)| f.clone())
                    .collect()
            };
            let mut files = files;
            files.dedup();
            (!files.is_empty()).then(|| (*s, files.join(" ")))
        })
        .collect()
}

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
    /// `mcp`, `git`, `app`, `owner` or `ci`.
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
    /// Its Secrets Store bindings.
    pub secrets_store: Vec<String>,
    /// The hostnames it serves, from its spec's routes; only `pigeon.dev`'s are named.
    pub hosts: Vec<Item>,
}

/// A box of the connection diagram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Part {
    /// Unique; a link's ends.
    pub id: String,
    /// `object`, `container`, `database`, `bucket`, `store`, `git`, `github` or `service`.
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
    /// Where macOS jobs run, first available first, from `.config/macos.yaml`.
    pub macos: Vec<Item>,
    /// Where the parts sit and how the links run.
    pub diagram: super::place::Diagram,
}

fn read(platform: &Path, file: &str) -> Option<String> {
    SOURCES
        .iter()
        .find_map(|s| std::fs::read_to_string(platform.join(s).join(file)).ok())
}

/// `file` from the root, under the first of [`SOURCES`] that holds it.
fn located(platform: &Path, file: &str) -> String {
    let s = SOURCES
        .iter()
        .find(|s| platform.join(s).join(file).is_file())
        .unwrap_or(&SOURCES[0]);
    format!("{s}/{file}")
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

/// Whether `krate` is the agent's crate.
fn is_agent(krate: &str) -> bool {
    krate.ends_with("pigeon-agent")
}

/// The network of the instance at `root`, served by the platform at `platform`; none when the
/// platform has no control Worker.
#[must_use]
pub fn network(root: &Path, platform: &Path) -> Option<Network> {
    let found = crate::workers::discover(platform)
        .ok()?
        .into_iter()
        .find(|f| is_agent(&f.spec.entry.krate) && !f.spec.color)?;
    let kind = found.spec.deployment.clone();
    let (manifest, _) = crate::infra::manifest::load(root, platform)?;
    let stacks: Vec<&crate::infra::manifest::Entry> = manifest
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
        Some(unit) => crate::workers::bind(root, &found, unit).ok()?.spec,
        None => found.spec,
    };
    let access = access(&stacks);
    let mut routes = routes(platform);
    routes.extend(own_routes(platform));
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
    let services = services(platform);
    let mut parts = parts(platform, &worker, &repos, github.as_deref());
    for (s, source) in &services {
        parts.push(Part {
            id: s.id.into(),
            kind: "service".into(),
            role: s.label.into(),
            many: false,
            source: source
                .split(' ')
                .map(|f| located(platform, f))
                .collect::<Vec<_>>()
                .join(" "),
        });
    }
    let known = |id: &str| id == "Worker" || parts.iter().any(|p| p.id == id);
    let mut links: Vec<Link> = LINKS
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
    links.extend(services.iter().map(|(s, source)| Link {
        id: format!("worker_{}", s.id.to_lowercase().replace(' ', "_")),
        from: "Worker".into(),
        to: s.id.into(),
        label: s.label.into(),
        kind: Kind::Call,
        source: source.clone(),
    }));
    let diagram = super::place::place(&parts, &links);
    Some(Network {
        clients: clients(&routes),
        access,
        worker,
        github,
        parts,
        links,
        macos: macos(platform),
        diagram,
    })
}

/// Where macOS jobs run, in `.config/macos.yaml`'s order, each with its provider's doc comment.
fn macos(platform: &Path) -> Vec<Item> {
    let Ok(config) = std::fs::read_to_string(platform.join(".config/macos.yaml")) else {
        return Vec::new();
    };
    let Some(order) = config.lines().find_map(|l| l.strip_prefix("order:")) else {
        return Vec::new();
    };
    let docs = read(platform, "core/macos/mod.rs").unwrap_or_default();
    let lines: Vec<&str> = docs.lines().collect();
    order
        .trim()
        .trim_matches(['[', ']'])
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .enumerate()
        .map(|(i, name)| {
            let mut variant = name.to_string();
            if let Some(f) = variant.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            let doc = lines
                .iter()
                .position(|l| l.trim() == format!("{variant},"))
                .and_then(|at| at.checked_sub(1))
                .and_then(|at| lines[at].trim().strip_prefix("///"))
                .map(|d| d.trim().trim_end_matches('.').replace('`', ""))
                .unwrap_or_default();
            let rank = if i == 0 { "first" } else { "failover" };
            item(name, format!("{rank} · {doc}"))
        })
        .collect()
}

/// The paths `fetch` serves before its router, from the string literals and `module::CONST`
/// paths in its body, by surface: `app` under the app's API, `ci` for the rest but health.
fn own_routes(platform: &Path) -> Vec<(String, Vec<String>)> {
    let Some(text) = read(platform, "worker/mod.rs") else {
        return Vec::new();
    };
    let Some(start) = text.find("async fn fetch(") else {
        return Vec::new();
    };
    let body = &text[start..];
    let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
    let mut paths = literals(body, "/");
    for c in consts_in(body) {
        if let Some(v) = const_value(platform, &c).filter(|v| v.starts_with('/')) {
            paths.push(v);
        }
    }
    let api = paths
        .iter()
        .find(|p| p.starts_with("/v0/app/"))
        .cloned()
        .unwrap_or_else(|| "/v0/app/".into());
    let mut app: Vec<String> = Vec::new();
    let mut ci = Vec::new();
    let mut health = Vec::new();
    for p in paths {
        let shown = if p.ends_with('/') && p.len() > 1 {
            format!("{p}…")
        } else {
            p.clone()
        };
        if p.starts_with(&api) || p.starts_with("/.well-known/") {
            app.push(shown);
        } else if p.starts_with("/healthz") {
            health.push(shown);
        } else {
            ci.push(shown);
        }
    }
    app.extend(app_api(platform, &api));
    let mut out = Vec::new();
    for (stem, mut v) in [("app", app), ("ci", ci), ("health", health)] {
        let mut seen = std::collections::BTreeSet::new();
        v.retain(|p| seen.insert(p.clone()));
        if !v.is_empty() {
            out.push((stem.to_string(), v));
        }
    }
    out
}

/// The app's API: each `"<api>…"` literal in the Worker's sources, with its method when a
/// `Method::` precedes it on its line.
fn app_api(platform: &Path, api: &str) -> Vec<String> {
    let dir = platform.join(SOURCES[0]).join("worker");
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
        let text = text.split("#[cfg(test)]").next().unwrap_or("");
        for line in text.lines() {
            let method = line.split("Method::").nth(1).map(|m| {
                m.chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect::<String>()
                    .to_uppercase()
            });
            for p in literals(line, api) {
                if p == api {
                    continue;
                }
                let p = if p.ends_with('/') {
                    format!("{p}…")
                } else {
                    p
                };
                out.push(match &method {
                    Some(m) => format!("{m} {p}"),
                    None => p,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        let path = |s: &str| s.rsplit(' ').next().unwrap_or(s).to_string();
        (path(a), a.clone()).cmp(&(path(b), b.clone()))
    });
    out.dedup();
    out
}

/// The string literals in `text` that start with `prefix`.
#[must_use]
pub fn literals(text: &str, prefix: &str) -> Vec<String> {
    text.split(&format!("\"{prefix}"))
        .skip(1)
        .filter_map(|s| s.find('"').map(|end| format!("{prefix}{}", &s[..end])))
        .filter(|p| !p.contains(['{', ' ', '\\']))
        .collect()
}

/// The `module::CONST` paths in `text`, such as `devices::API` or `crate::apple::ROUTE`.
fn consts_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("::") {
        let before = &rest[..i];
        let module: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_lowercase() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let after = &rest[i + 2..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || *c == '_')
            .collect();
        let next = after[name.len()..].chars().next();
        if !module.is_empty()
            && name.len() > 1
            && !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '(' || c == ':')
        {
            out.push(format!("{module}::{name}"));
        }
        rest = after;
    }
    out.sort();
    out.dedup();
    out
}

/// The value of `pub const <NAME>: &str = "…"` in `<module>.rs`, under `worker/` or the crate.
fn const_value(platform: &Path, path: &str) -> Option<String> {
    let (module, name) = path.split_once("::")?;
    let decl = format!("const {name}: &str = \"");
    [format!("worker/{module}.rs"), format!("{module}.rs")]
        .iter()
        .filter_map(|f| read(platform, f))
        .find_map(|text| {
            let at = text.find(&decl)? + decl.len();
            let end = text[at..].find('"')?;
            Some(text[at..at + end].to_string())
        })
}

/// Each surface's routes, by its file's stem, as `METHOD path`; a prefix ends in `…`.
fn routes(platform: &Path) -> Vec<(String, Vec<String>)> {
    let dir = platform.join(SOURCES[0]).join("worker");
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
    let mut app = Vec::new();
    let mut ci = Vec::new();
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
                "app" => app.push(item(p.clone(), "")),
                "ci" => ci.push(item(p.clone(), "")),
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
    // A page's sub-paths and scripts are said beside it, not listed.
    let parent = |p: &str| -> Option<String> {
        let me = path(p);
        owner
            .iter()
            .map(|o| path(o))
            .filter(|o| *o != me && me.starts_with(&format!("{}/", o.trim_end_matches('/'))))
            .max_by_key(String::len)
    };
    let pages: Vec<Item> = owner
        .iter()
        .filter(|p| parent(p).is_none())
        .map(|p| {
            let me = path(p);
            let under: Vec<String> = owner
                .iter()
                .filter(|o| parent(o).as_deref() == Some(me.as_str()))
                .map(|o| path(o))
                .collect();
            item(p.clone(), under.join(" "))
        })
        .collect();
    [
        ("mcp", "MCP clients", mcp),
        ("git", "git", git),
        ("app", "iOS app", app),
        ("owner", "Owner's browser", pages),
        ("ci", "CI and job Containers", ci),
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

/// The Worker's routes, by client: the diagram's Clients row; see .claude/decisions/2026-11-15-runtime-one-mcp-entry-point.md §7.
#[must_use]
pub fn inventory(platform: &Path) -> Vec<Client> {
    let mut r = routes(platform);
    r.extend(own_routes(platform));
    clients(&r)
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
    let dir = platform.join(SOURCES[0]).join("worker");
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
            "{}/worker/{}",
            SOURCES[0],
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
    if !w.secrets_store.is_empty() {
        out.push(part(
            "Secrets Store",
            "store",
            w.secrets_store.join(" · "),
            false,
            spec,
        ));
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
fn access(stacks: &[&crate::infra::manifest::Entry]) -> Access {
    let mut access = Access::default();
    for e in stacks {
        let strings = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        };
        access
            .hosts
            .extend(strings(&e.settings["domains"]).iter().map(|h| shown(h)));
        for t in ["service_tokens", "adopted_service_tokens"]
            .iter()
            .filter_map(|k| e.settings[*k].as_array())
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

/// What a standalone shows instead of its hostname or account.
pub const STANDALONE: &str = "standalone instance (own account)";

/// `host` when it is `pigeon.dev`'s, else [`STANDALONE`]; no other zone is ever named.
fn shown(host: &str) -> String {
    if host == "pigeon.dev" || host.ends_with(".pigeon.dev") {
        host.to_string()
    } else {
        STANDALONE.to_string()
    }
}

/// The route keys [`hosts`] represents; the coverage test fails on any other.
pub const ROUTE_KEYS: &[&str] = &["custom_domain", "subdomains_of"];

/// The hostnames the spec's routes serve, each with what it is.
fn hosts(spec: &crate::workers::Spec) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for r in &spec.routes {
        let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(r) else {
            continue;
        };
        for (k, v) in fields {
            let Some(v) = v.as_str() else { continue };
            match k.as_str() {
                "custom_domain" => out.push(item(shown(v), "custom domain")),
                "subdomains_of" => out.push(item(
                    if shown(v) == STANDALONE {
                        STANDALONE.to_string()
                    } else {
                        format!("*.{v}")
                    },
                    format!("every subdomain: each {STANDALONE}"),
                )),
                _ => {}
            }
        }
    }
    out.dedup();
    out
}

/// The Worker, from its bound spec.
fn worker(
    platform: &Path,
    spec: &crate::workers::Spec,
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
        secrets_store: spec.bindings.secrets_store.keys().cloned().collect(),
        hosts: hosts(spec),
    }
}
