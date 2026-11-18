//! The operation catalog and its wire form, shared by the `engines` and `worker` builds
//! (control-plane ADR §1).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::view::View;

pub mod artifacts_wire;
pub mod link;
pub mod results;
pub mod ship_wire;

/// Why an operation could not run.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read.
    Io {
        /// The path.
        path: PathBuf,
        /// Why.
        message: String,
    },
    /// The manifest or a request is invalid; every problem found.
    Invalid(Vec<String>),
    /// An engine failed.
    Engine {
        /// The engine.
        engine: Engine,
        /// Its error.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Invalid(problems) => {
                write!(f, "{} problem(s):", problems.len())?;
                for p in problems {
                    write!(f, "\n{p}")?;
                }
                Ok(())
            }
            Self::Engine { engine, message } => write!(f, "{}: {message}", engine.name()),
        }
    }
}

impl std::error::Error for Error {}

/// Who runs an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// The agent itself.
    Agent,
    /// Policy and its enforcement.
    Eviction,
    /// Actions: infrastructure, releases.
    Infra,
}

impl Engine {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Eviction => "eviction",
            Self::Infra => "infra",
        }
    }
}

/// What an operation may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Reads only.
    Read,
    /// Computes changes without making them.
    Plan,
    /// Makes changes.
    Apply,
    /// Writes only its own work directory; changes nothing deployed.
    Build,
}

impl Effect {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Plan => "plan",
            Self::Apply => "apply",
            Self::Build => "build",
        }
    }
}

/// An argument's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A string.
    String,
    /// A string, number or boolean, read as text.
    Text,
    /// A list of strings.
    Strings,
    /// A number.
    Number,
    /// A boolean.
    Bool,
    /// Any value.
    Any,
    /// A list of objects with these fields.
    List(&'static [Arg]),
    /// An object with these fields.
    Object(&'static [Arg]),
}

impl Kind {
    fn accepts(self, v: &Value) -> bool {
        match self {
            Self::String => v.is_string(),
            Self::Text => v.is_string() || v.is_number() || v.is_boolean(),
            Self::Strings => v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
            Self::Number => v.is_number(),
            Self::Bool => v.is_boolean(),
            Self::Any => true,
            Self::List(_) => v.is_array(),
            Self::Object(_) => v.is_object(),
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::String => "a string",
            Self::Text => "text",
            Self::Strings => "a list of strings",
            Self::Number => "a number",
            Self::Bool => "a boolean",
            Self::Any => "any value",
            Self::List(_) => "a list of objects",
            Self::Object(_) => "an object",
        }
    }

    /// Its JSON Schema.
    #[must_use]
    pub fn schema(self) -> Value {
        match self {
            Self::String => json!({ "type": "string" }),
            Self::Text => json!({ "type": ["string", "number", "boolean"] }),
            Self::Strings => json!({ "type": "array", "items": { "type": "string" } }),
            Self::Number => json!({ "type": "number" }),
            Self::Bool => json!({ "type": "boolean" }),
            Self::Any => json!({}),
            Self::List(f) => json!({ "type": "array", "items": object_schema(f) }),
            Self::Object(f) => object_schema(f),
        }
    }
}

/// One argument an operation takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Arg {
    /// Its name.
    pub name: &'static str,
    /// Whether every request names it.
    pub required: bool,
    /// Its type.
    pub kind: Kind,
}

/// An open object schema: these fields' types, none required, others allowed.
fn object_schema(fields: &[Arg]) -> Value {
    let properties: Map<String, Value> = fields
        .iter()
        .map(|a| (a.name.to_string(), a.kind.schema()))
        .collect();
    json!({ "type": "object", "properties": properties })
}

pub(crate) const fn arg(name: &'static str, required: bool, kind: Kind) -> Arg {
    Arg {
        name,
        required,
        kind,
    }
}

/// Who answers an operation's request for approval; see .claude/decisions/2026-11-15-runtime-one-mcp-entry-point.md §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Asks {
    /// Runs on the caller's capability.
    None,
    /// A mandate, window, policy or change grant answers, else pending for the owner.
    Grant,
    /// The owner proves on its page, each time.
    Page,
}

/// A door an operation is reachable through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Door {
    /// `/mcp`, behind Access.
    Access,
    /// `/v0/app/mcp`, by device signature.
    Device,
}

const GRANT: [&str; 7] = [
    "change.authorize",
    "change.approve",
    "change.merge",
    "change.close",
    crate::control::BREAK_GLASS,
    "change.pin",
    "change.front",
];

const PAGE: [&str; 8] = [
    "mandate.propose",
    "lane.request",
    "ci.window",
    "pool.resume",
    "grant.request",
    "grant.rotate",
    "grant.revoke",
    "platform.apply",
];

/// One operation a surface may request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Operation {
    /// Its dotted name, `<area>.<verb>`.
    pub name: &'static str,
    /// Who runs it.
    pub engine: Engine,
    /// What it may change.
    pub effect: Effect,
    /// What it does.
    pub summary: &'static str,
    /// The arguments it takes.
    pub args: &'static [Arg],
    /// The view its result names; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.
    pub view: crate::view::View,
}

impl Operation {
    /// Its arguments as a JSON Schema object, for MCP's `inputSchema`.
    #[must_use]
    pub fn input_schema(&self) -> Value {
        let properties: Map<String, Value> = self
            .args
            .iter()
            .map(|a| (a.name.to_string(), a.kind.schema()))
            .collect();
        let required: Vec<&str> = self
            .args
            .iter()
            .filter(|a| a.required)
            .map(|a| a.name)
            .collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    /// Its MCP outputSchema: the response envelope, with its declared result; none when it declares none.
    #[must_use]
    pub fn output_schema(&self) -> Option<Value> {
        results::result(self.name).map(|f| {
            json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string" },
                    "ok": { "type": "boolean" },
                    "result": object_schema(f),
                    "violations": { "type": "array", "items": { "type": "string" } },
                },
                "required": ["operation", "ok"],
            })
        })
    }

    /// Whether only the control plane runs it, from its Durable Objects and D1.
    #[must_use]
    pub fn control_plane(&self) -> bool {
        self.name.starts_with("change.")
            || self.name.starts_with("ci.")
            || self.name.starts_with("deploy.")
            || self.name.starts_with("build.")
            || self.name.starts_with("git.")
            || self.name.starts_with("mandate.")
            || self.name.starts_with("lane.")
            || self.name.starts_with("task.")
            || self.name.starts_with("brief.")
            || self.name.starts_with("events.")
            || self.name.starts_with("feed.")
            || self.name.starts_with("digest.")
            || self.name.starts_with("home.")
            || self.name.starts_with("tracker.")
            || self.name.starts_with("ops.")
            || self.name.starts_with("insights.")
            || self.name.starts_with("session.")
            || self.name.starts_with("platform.")
            || self.name.starts_with("apple.")
            || self.name.starts_with("ios.")
            || self.name.starts_with("grant.")
            || self.name.starts_with("owner.")
            || self.name.starts_with("provider.")
            || self.name.starts_with("bench.")
            || self.name.starts_with("pool.")
    }

    /// Who answers its request for approval.
    #[must_use]
    pub fn asks(&self) -> Asks {
        if GRANT.contains(&self.name) {
            Asks::Grant
        } else if PAGE.contains(&self.name) {
            Asks::Page
        } else {
            Asks::None
        }
    }

    /// The capability a caller needs to run it.
    #[must_use]
    pub fn capability(&self) -> &'static str {
        if self.effect == Effect::Read {
            "read"
        } else if self.effect == Effect::Apply && !self.control_plane() {
            "platform"
        } else {
            "propose"
        }
    }

    /// The doors it is reachable through.
    #[must_use]
    pub fn doors(&self) -> &'static [Door] {
        &[Door::Access]
    }
}

const PR_FIELDS: &[Arg] = &[
    arg("title", false, Kind::Text),
    arg("body", false, Kind::Text),
    arg("number", false, Kind::Text),
    arg("files", false, Kind::Text),
    arg("branch", false, Kind::Text),
    arg("scope", false, Kind::Text),
    arg("test_reason", false, Kind::Text),
    arg("diff", false, Kind::Text),
];

const NUMBER: Arg = arg("number", true, Kind::Text);

const CATALOG: &[Operation] = &[
    Operation {
        name: "instance.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Show the instance manifest",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "architecture.describe",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Describe the instance's architecture: its graph, layout and network, each part with the code it was derived from, as of the deployed SHA (args: format?, json, text or svg)",
        args: &[arg("format", false, Kind::Text)],
        view: View::Detail,
    },
    Operation {
        name: "policy.check",
        engine: Engine::Eviction,
        effect: Effect::Read,
        summary: "Check the instance against its policies (args: title?, body?, number?, files?, which add the pull-request policy; branch?, scope?, test_reason? and diff? add the change policy)",
        args: PR_FIELDS,
        view: View::Detail,
    },
    Operation {
        name: "infra.validate",
        engine: Engine::Infra,
        effect: Effect::Read,
        summary: "Validate every infrastructure and workload unit",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "change.scope",
        engine: Engine::Infra,
        effect: Effect::Read,
        summary: "The checks a change needs, from the paths it touches, in the order to run them, and the units a merge of them deploys (`deploys`) (args: files, one path per line)",
        args: &[arg("files", true, Kind::Text)],
        view: View::Detail,
    },
    Operation {
        name: "change.propose",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Propose a change: its slug, Conventional Commit title, plan and scope (globs, one per line), and optionally the `mandate` it is carried out under, the `session` proposing it, for a design change the globs its ADR `affects`, `full` true to run the whole test suite in its merge gate, `resumes` an evicted change's number to finish it, `after` an open change's number to build on its branch (the new change waits for it, and restacks onto main when it merges), and `asks`, JSON `[{key, label, help?, chars?: hex|lower|alnum|token, min?, max?}]`, values the owner types at the plan's approval, public by design; `brief` (`b<n>`) for a change carrying out a brief, whose title, plan and scope then come from it and which an executor (haiku or sonnet) must name; title, plan and scope are required otherwise; returns its number, its branch, change/<number>-<slug>, and the open changes it `overlaps`",
        args: &[
            arg("slug", true, Kind::String),
            arg("title", false, Kind::String),
            arg("plan", false, Kind::String),
            arg("scope", false, Kind::String),
            arg("mandate", false, Kind::Text),
            arg("session", false, Kind::String),
            arg("affects", false, Kind::String),
            arg("ticket", false, Kind::String),
            arg("full", false, Kind::Text),
            arg("resumes", false, Kind::Text),
            arg("after", false, Kind::Text),
            arg("asks", false, Kind::Text),
            arg("brief", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "change.pin",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "The owner's: pin an open change against eviction, or with `off` true unpin it",
        args: &[
            NUMBER,
            arg("off", false, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "change.front",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "The owner's: move a queued change to the front of the merge queue, behind the merge running; it skips no check",
        args: &[NUMBER, arg("ticket", false, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "change.authorize",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "The owner's or a reviewer's approval of a proposed change's plan, so its branch may be pushed; never by its proposer. Under the owner's Auto window or mandate the control plane authorizes it at proposal, so a session doesn't call this",
        args: &[NUMBER, arg("ticket", false, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "change.pushed",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Report a push of a change's branch, naming its head, when the push event may be missed; the bound job refuses a SHA that isn't the branch's head",
        args: &[
            NUMBER,
            arg("sha", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "change.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "A change's state, head, approvals, violations, each job's result, and its stage: `stage`, who `needs` to act, `waits_on`, `next` and `since`",
        args: &[NUMBER],
        view: View::Detail,
    },
    Operation {
        name: "change.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Every change, newest first, each with its stage and its session's model class (`model`) (args: state?)",
        args: &[arg("state", false, Kind::String)],
        view: View::List,
    },
    Operation {
        name: "change.evictable",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Each open change as an eviction candidate: the fields an eviction policy judges (no change is evicted yet)",
        args: &[],
        view: View::List,
    },
    Operation {
        name: "change.approve",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "The owner's or a reviewer's approval of a checked change's head, which queues its merge (sha?: the head; a person's call without it approves the current head; hold?: true approves without queuing the merge); never by its proposer. A session never calls it on its own change: it asks to merge (`change_merge`), and the owner's window, mandate or policy approves in the control plane",
        args: &[
            NUMBER,
            arg("sha", false, Kind::String),
            arg("hold", false, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "change.merge",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "Ask to merge your change: an approved head is squashed onto main as `<title> (!<number>)` and its branch deleted; a checked head is first approved by the owner's window, mandate or policy that covers it, else waits, listed, for the owner, whose approval queues the merge (args: full?, true to run the whole test suite in its merge gate)",
        args: &[
            NUMBER,
            arg("ticket", false, Kind::String),
            arg("full", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "change.close",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "Close a change unmerged, stopping its jobs. Under Auto, a session may close its own unapproved change, as when it is superseded or refused. A session never closes a generated change (`reconcile-…`, `review-…`): `task_done` on its task closes it, and the control plane retires it once its reason is gone",
        args: &[NUMBER, arg("ticket", false, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "ci.rerun",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Run one of a finished head's jobs again, and those its failure cancelled; `on: cloudflare` asks the owner to run the head's code checks on Cloudflare instead of GitHub",
        args: &[
            NUMBER,
            arg("job", true, Kind::String),
            arg("on", false, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "ci.cloudflare",
        engine: Engine::Agent,
        effect: Effect::Apply,
        summary: "The owner's break-glass: run a change head's code checks that haven't passed on Cloudflare's Checker instead of GitHub Actions",
        args: &[NUMBER, arg("ticket", false, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "ci.window",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Ask the owner to run every change's code checks on Cloudflare's Checker instead of GitHub Actions for `hours` (1 to 24; 0 closes the window early); answers with the page's link, granted with a passkey",
        args: &[
            arg("hours", true, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "lane.request",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Ask the owner for a fast lane: for `minutes` (default 30, at most 120), the changes of one line of work, `session` (a session's URL), `changes` (numbers, one per line) or `paths` (globs, one per line), have their code checks waived and go to the front of the merge queue; `reason` is required, and `extends` names an open lane to widen or extend; answers with the page's link, granted with a passkey",
        args: &[
            arg("session", false, Kind::String),
            arg("changes", false, Kind::Text),
            arg("paths", false, Kind::String),
            arg("minutes", false, Kind::Text),
            arg("reason", true, Kind::String),
            arg("extends", false, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "lane.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The open fast lane and the last ten: each one's line, reason, end, the jobs it waived and the changes merged under it",
        args: &[],
        view: View::List,
    },
    Operation {
        name: "lane.end",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "End fast lane `id` now, saying `why`; open to the owner and to the sessions the lane names, since it only takes power away",
        args: &[
            arg("id", true, Kind::Text),
            arg("why", false, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "pool.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Each capacity pool's state and ledger: its windows' spend, reservations, running tasks and whether it is paused (args: pool?, one pool's name)",
        args: &[arg("pool", false, Kind::String)],
        view: View::Detail,
    },
    Operation {
        name: "pool.pause",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Pause capacity pool `pool`, saying `why`: it admits no new task until the owner resumes it; open to anyone, since it only takes power away",
        args: &[
            arg("pool", true, Kind::String),
            arg("why", true, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "pool.resume",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Ask the owner to resume paused capacity pool `pool`; answers with the page's link, granted with a passkey",
        args: &[
            arg("pool", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "grant.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The grants: the deployed ledger, and each grant's route, token ID, capabilities and expiry, never a value",
        args: &[],
        view: View::List,
    },
    Operation {
        name: "grant.request",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Ask the owner to mint or generate grant `name` from the deployed ledger and put it at its route; answers with the page's link, where a passkey runs it",
        args: &[
            arg("name", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "grant.rotate",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Ask the owner to make grant `name` anew, put and verify it, then revoke the old; answers with the page's link",
        args: &[
            arg("name", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "grant.revoke",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Ask the owner to revoke grant `name` and remove its value; answers with the page's link",
        args: &[
            arg("name", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "git.tokens",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The repository's git tokens: each one's ID, scope, state and expiry, never its value",
        args: &[],
        view: View::List,
    },
    Operation {
        name: "sites.build",
        engine: Engine::Infra,
        effect: Effect::Build,
        summary: "Build static sites into the work directory with the instance's architecture as data; publishes nothing (args: sites?)",
        args: &[arg("sites", false, Kind::Strings)],
        view: View::Card,
    },
    Operation {
        name: "sites.publish",
        engine: Engine::Infra,
        effect: Effect::Apply,
        summary: "Build the named sites, or every site, with the architecture as data, sync each to its bucket and check it live, with the run's CLOUDFLARE_API_TOKEN (args: sites?, deployed?, the SHA the deploy runs for)",
        args: &[
            arg("sites", false, Kind::Strings),
            arg("deployed", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "deploy.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "What main's pipeline last published: the SHA deployed, the one in progress, and recent deploys with their logs",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "build.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The files stored for commit `sha`'s build (pigeon, the judge, the test map), each with its size; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §6",
        args: &[arg("sha", true, Kind::String)],
        view: View::List,
    },
    Operation {
        name: "build.read",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Up to 4 MiB of build file `name` of commit `sha` from byte `offset` (0 by default), base64, with the whole file's `size` and the `next` offset, null at its end; check the whole file against deploy.status's sha256; see the ADR §6",
        args: &[
            arg("sha", true, Kind::String),
            arg("name", true, Kind::String),
            arg("offset", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "home.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The app's home: today's cost and error tiles, the approvals waiting, Auto's state and the owner's open items; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md step 11",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "tracker.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The tracker: open changes by stage and session, and those merged lately; see the ADR step 11",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "ops.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Operations over `range` days (1, 7 or 30; 1 by default): merges, checks, deploys and their times; see the ADR step 11",
        args: &[arg("range", false, Kind::Text)],
        view: View::Detail,
    },
    Operation {
        name: "insights.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "One source's stacks of `metric` (`cost` or `errors`) by kind over `range` days (1 by default); `source` is cloudflare, github, bitrise or buildkite; see the ADR step 11",
        args: &[
            arg("metric", true, Kind::String),
            arg("source", true, Kind::String),
            arg("range", false, Kind::Text),
        ],
        view: View::Detail,
    },
    Operation {
        name: "approval.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "What waits for the owner: the open fast lanes, then every approval, grant and offer waiting, newest first, each opening its proof; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md, its 2026-11-18 amendment §4",
        args: &[],
        view: View::List,
    },
    Operation {
        name: "mandate.propose",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Propose a mandate: a task the owner grants once with a passkey, so changes inside it go through without asking; `adr` (the ADR it executes), `summary`, `scope` (globs, one per line), `may_deploy`, `budget` (merges, 1 to 10, default 5) and `hours` (1 to 72, default 24); answers with the page's link",
        args: &[
            arg("adr", true, Kind::String),
            arg("summary", true, Kind::String),
            arg("scope", true, Kind::String),
            arg("may_deploy", false, Kind::Text),
            arg("budget", false, Kind::Text),
            arg("hours", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "ios.release",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Release main's commit on GitHub to TestFlight as the next iOS build: archived on GitHub, signed on Buildkite, processed by App Store Connect; one at a time",
        args: &[],
        view: View::Card,
    },
    Operation {
        name: "ios.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The iOS release in flight, its stage and links, and the newest releases with their states",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "apple.check",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Whether the Apple connection works: its secrets are set, App Store Connect accepts its tokens, the app exists, and its App Store profile is active, with its expiry; it never shows a secret",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "digest.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The owner's digest now: approvals waiting, tasks unclaimed two hours, paused mandates, failed deploys and changes merged without the owner; `needs_owner` says whether it would be pushed",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "session.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Session `session`'s class (`model`, `role`, and an executor's `planner` and `brief`), its open changes, oldest first, each with its stage (`stage`, `needs`, `waits_on`, `next`, `since`), its tasks, and its event cursor: `after`, `polled` and whether it is `listening` (polled within two minutes)",
        args: &[arg("session", true, Kind::String)],
        view: View::Detail,
    },
    Operation {
        name: "session.end",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "End session `label`'s ticket, so the label works as before; open to anyone, since it only takes power away. Answers what each change it holds has left, whether the end is `clean` (nothing held but hand-offs), and its `archiver`, which a clean end wakes with `session.archivable`",
        args: &[arg("label", true, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "session.archived",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As archiver `session`, report on ended session `label`, after `session.archivable`: `outcome` `archived` or `skipped`, and `why`; accepted only from its archiver, with its `ticket`, after a clean end, and once",
        args: &[
            arg("label", true, Kind::String),
            arg("session", true, Kind::String),
            arg("outcome", true, Kind::String),
            arg("why", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "bench.run",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Run `argv` on your bench `bench` (0 to 2, default 0) at `sha`, the commit `pigeon session bench push` put on your scratch ref: an allowed `pigeon dev <task>`, `cargo fmt|check|clippy|build|test|tree|metadata …`, `bench check <name>`, `pigeon infra validate|render …`, `pigeon dev scope …`, `pigeon decisions …`, or `git diff|log|status …`; answers its `run`; one run per bench at a time; `timeout` in minutes, 1 to 90",
        args: &[
            arg("ticket", true, Kind::String),
            arg("sha", true, Kind::String),
            arg("argv", true, Kind::Strings),
            arg("bench", false, Kind::Text),
            arg("timeout", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "bench.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Run `run`'s state, exit code, seconds and disk on your bench `bench`, with its log from `offset` (up to 64 KB, and `next`), waiting up to `wait` seconds (at most 25) for new output",
        args: &[
            arg("ticket", true, Kind::String),
            arg("run", true, Kind::String),
            arg("bench", false, Kind::Text),
            arg("offset", false, Kind::Text),
            arg("wait", false, Kind::Text),
        ],
        view: View::Detail,
    },
    Operation {
        name: "bench.logs",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "As bench.status: run `run`'s log from `offset` on your bench `bench`, waiting up to `wait` seconds",
        args: &[
            arg("ticket", true, Kind::String),
            arg("run", true, Kind::String),
            arg("bench", false, Kind::Text),
            arg("offset", false, Kind::Text),
            arg("wait", false, Kind::Text),
        ],
        view: View::Detail,
    },
    Operation {
        name: "bench.cancel",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Cancel run `run` on your bench `bench`",
        args: &[
            arg("ticket", true, Kind::String),
            arg("run", true, Kind::String),
            arg("bench", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "bench.reset",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Reset your bench `bench`: its untracked files and the workspace crates' build; `hard` true destroys its Container instead",
        args: &[
            arg("ticket", true, Kind::String),
            arg("bench", false, Kind::Text),
            arg("hard", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "bench.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Your benches: each one's name, whether it runs a command, and when it was last used; and your place if you wait for a first bench",
        args: &[arg("ticket", true, Kind::String)],
        view: View::List,
    },
    Operation {
        name: "bench.warm",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Start your bench `bench` (default 0) without running anything, so the first run is warm",
        args: &[
            arg("ticket", true, Kind::String),
            arg("bench", false, Kind::Text),
        ],
        view: View::Card,
    },
    Operation {
        name: "owner.ask",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As `session`, flag the owner for what only they can settle: `kind` `decision` (a choice only the owner can make, with up to 4 `options`, one per line) or `action` (something only they can do: provider setup, a credential, account recovery, an edit to a guarded file), with a one-line `title`, its `detail` and the `change` it is about; pushed, in the digest and the app's Approvals tab until answered. `owner.answered` wakes you with the answer; stop only the work that depends on it. Never for a choice the rules settle, or a destructive act, which is asked where it happens",
        args: &[
            arg("kind", true, Kind::String),
            arg("title", true, Kind::String),
            arg("detail", true, Kind::Text),
            arg("session", true, Kind::String),
            arg("change", false, Kind::Text),
            arg("options", false, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "owner.withdraw",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As `session`, withdraw your own open owner item `id`, as when its action is seen done; only the owner answers an item, from their device",
        args: &[
            arg("id", true, Kind::Text),
            arg("session", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "owner.answer",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "The owner answers their open item `id` with `answer`: one of a decision's options, or `done` for an action; a person's only (signed in, or on a linked device), never a service token or an agent; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md step 11",
        args: &[
            arg("id", true, Kind::Text),
            arg("answer", true, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "provider.call",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Call tool `tool` of provider MCP server `server` (`.config/providers/mcp.yaml`) with `arguments`, JSON text of an object, for `change` if named; the broker checks the registry, your capability, ticket and change, and the tool's tier before it goes out, and records provider.called or provider.refused",
        args: &[
            arg("server", true, Kind::String),
            arg("tool", true, Kind::String),
            arg("arguments", false, Kind::Text),
            arg("change", false, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "provider.tools",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Provider MCP server `server`'s live tools beside the registry's pins: each tool's name, live schema hash, pinned hash and whether it drifted",
        args: &[
            arg("server", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::List,
    },
    Operation {
        name: "session.start",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "A ticket for session `label` (its URL), lasting 24 hours; pass it as `ticket` on every change it proposes and acts on, and as the X-Pigeon-Ticket header on its pushes. A label already holding a live ticket is refused unless `replace`, which revokes that one. `model` is the session's class, `haiku`, `sonnet` or `opus` (the default); an executor (haiku or sonnet) names its `planner`'s label and its `brief` (`b<n>`); `parent` is the label of the session that created it, which archives it once it ends clean",
        args: &[
            arg("label", true, Kind::String),
            arg("replace", false, Kind::Text),
            arg("model", false, Kind::String),
            arg("planner", false, Kind::String),
            arg("brief", false, Kind::String),
            arg("parent", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "events.wait",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The instance's events after `after` (an event number, or `session` to resume from the session's cursor, which each numbered poll moves forward), for `session` and `change` (numbers, comma-separated) if named, only those the agent must act on when `wake` is `agent`, waiting up to `seconds` (at most 25) for one; answers `last` and `events`",
        args: &[
            arg("after", false, Kind::Text),
            arg("session", false, Kind::String),
            arg("change", false, Kind::Text),
            arg("wake", false, Kind::String),
            arg("seconds", false, Kind::Text),
        ],
        view: View::List,
    },
    Operation {
        name: "feed.read",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The feed's items after `after` (an item number, or `session` to resume from the session's kept cursor), for `session` if named, only those needing `needs` (`owner` or `agent`) when named, only the open ones when `open` is `1`, only those that wake their session when `wake` is `agent`, waiting up to `seconds` (at most 25) for one; answers `last` and `items`; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §3",
        args: &[
            arg("after", false, Kind::Text),
            arg("session", false, Kind::String),
            arg("needs", false, Kind::String),
            arg("open", false, Kind::Text),
            arg("wake", false, Kind::String),
            arg("seconds", false, Kind::Text),
        ],
        view: View::List,
    },
    Operation {
        name: "brief.create",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As opus planner `session`, store `draft`, a brief as JSON text (title, adr, step, model `haiku` or `sonnet`, base, scope, decisions, steps of do/files/content/done_when, checks, stop_if, budget?, commit) as brief `b<n>`; refused with every problem when it leaves the executor a choice",
        args: &[
            arg("session", true, Kind::String),
            arg("draft", true, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "brief.amend",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As brief `id`'s planner `session`, append `draft` as its next revision, saying why (`reason`); earlier revisions are never rewritten",
        args: &[
            arg("id", true, Kind::String),
            arg("session", true, Kind::String),
            arg("draft", true, Kind::Text),
            arg("reason", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "brief.review",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As the planner `session` of the brief change `change` carries out: `verdict` `accept` (with the `head` reviewed; only then may a window, mandate or policy approve it, and under a window the control plane approves it at once, so don't call `change_approve`), `revise` (`notes` go to the executor; amend the brief with `brief_amend`), or `take` (the change is offered back to you)",
        args: &[
            arg("change", true, Kind::Text),
            arg("verdict", true, Kind::String),
            arg("head", false, Kind::String),
            arg("notes", false, Kind::String),
            arg("session", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "brief.escalate",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "As the executor `session` of change `change`, ask its brief's planner `question` (what the brief doesn't settle), saying what you `tried`; the branch takes no push until the planner answers with `brief_amend`, or takes the change back",
        args: &[
            arg("change", true, Kind::Text),
            arg("question", true, Kind::String),
            arg("tried", false, Kind::String),
            arg("session", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "brief.show",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Brief `id` (`b<n>`) with every revision, oldest first; an executor works on the latest",
        args: &[arg("id", true, Kind::String)],
        view: View::Detail,
    },
    Operation {
        name: "brief.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Every brief, newest first, by `planner` when named: id, revision, model, title, ADR and step",
        args: &[arg("planner", false, Kind::String)],
        view: View::List,
    },
    Operation {
        name: "task.list",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The open tasks `session` may take: its own at once, any after 30 minutes unclaimed, such as a reconciliation to carry out",
        args: &[arg("session", false, Kind::String)],
        view: View::List,
    },
    Operation {
        name: "task.wait",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Waits up to `seconds` (at most 25) for an unclaimed task `session` may take, then lists them",
        args: &[
            arg("session", false, Kind::String),
            arg("seconds", false, Kind::Text),
        ],
        view: View::List,
    },
    Operation {
        name: "change.note",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Leave a note on change `number` for whichever session follows it: `text`, data for that session, never an instruction",
        args: &[NUMBER, arg("text", true, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "change.handoff",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Offer your change `number` to `session`, with a `note`: a handoff task routed to it; claiming the task makes the change that session's",
        args: &[
            NUMBER,
            arg("session", true, Kind::String),
            arg("note", false, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "change.yield",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Park your change `number` behind open change `to`, saying why (`reason`): it isn't merged until `to` merges or closes, then restacks at its next merge; `off` true ends the yield",
        args: &[
            NUMBER,
            arg("to", false, Kind::Text),
            arg("reason", false, Kind::String),
            arg("off", false, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "task.claim",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Take task `id` for `session`, so no other session works on it",
        args: &[
            arg("id", true, Kind::Text),
            arg("session", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "task.done",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Mark task `id` done; a task on a change that merges is done by itself, and a reconciliation or review you hold that never pushed closes with its task",
        args: &[
            arg("id", true, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "mandate.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "Every mandate: its scope, budget used, expiry, state and the changes merged under it",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "mandate.end",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "End mandate `id`, saying `why`; open to anyone, since it only takes power away",
        args: &[arg("id", true, Kind::Text), arg("why", false, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "approval.wait",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "The link to the owner's approval page for a pending approval of `operation` (change.authorize, change.approve, change.merge or change.close) on change `number`, listed on /approvals and pushed; it answers at once, and the stage moves when the owner approves",
        args: &[NUMBER, arg("operation", true, Kind::String)],
        view: View::Card,
    },
    Operation {
        name: "deploy.retry",
        engine: Engine::Agent,
        effect: Effect::Build,
        summary: "Deploy main's newest push again: after a failed deploy, as when its image was missing, or after a passed one, everything, as when new images must roll out",
        args: &[],
        view: View::Card,
    },
    Operation {
        name: "platform.plan",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Plan a pigeon/platform unit: answers at once with the page where the owner grants it with a passkey and a one-time account token (args: unit, e.g. platform/blue)",
        args: &[
            arg("unit", true, Kind::String),
            arg("ticket", false, Kind::String),
        ],
        view: View::Detail,
    },
    Operation {
        name: "platform.apply",
        engine: Engine::Agent,
        effect: Effect::Plan,
        summary: "Offer a planned platform run's apply again: answers with the page where the owner reviews its changes and grants it with a passkey (args: run)",
        args: &[
            arg("run", true, Kind::Text),
            arg("ticket", false, Kind::String),
        ],
        view: View::Card,
    },
    Operation {
        name: "platform.status",
        engine: Engine::Agent,
        effect: Effect::Read,
        summary: "The platform runs, newest first, with nothing secret",
        args: &[],
        view: View::Detail,
    },
    Operation {
        name: "infra.plan",
        engine: Engine::Infra,
        effect: Effect::Plan,
        summary: "Plan units and store the plan for 24 hours (args: units?; state, which serve supplies)",
        args: &[
            arg("state", true, Kind::String),
            arg("units", false, Kind::Strings),
        ],
        view: View::Detail,
    },
    Operation {
        name: "infra.apply",
        engine: Engine::Infra,
        effect: Effect::Apply,
        summary: "Apply a stored, unexpired plan exactly (args: plan; state, which serve supplies)",
        args: &[
            arg("state", true, Kind::String),
            arg("plan", true, Kind::String),
        ],
        view: View::Card,
    },
];

/// Every operation.
#[must_use]
pub fn catalog() -> &'static [Operation] {
    CATALOG
}

/// A request for one operation.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The operation's name.
    pub operation: String,
    /// Its arguments, as [`catalog`]'s summaries name them.
    #[serde(default)]
    pub args: Map<String, Value>,
}

/// An operation's outcome; the wire form every surface shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The operation's name.
    pub operation: String,
    /// Whether it found no violation.
    pub ok: bool,
    /// What it returned.
    pub result: Value,
    /// Every violation found.
    pub violations: Vec<String>,
}

/// Checks a request's operation, argument names and argument types without running it.
///
/// # Errors
///
/// [`Error::Invalid`] naming every problem.
pub fn validate(request: &Request) -> Result<&'static Operation, Error> {
    let Some(op) = CATALOG.iter().find(|o| o.name == request.operation) else {
        return Err(Error::Invalid(vec![format!(
            "no operation `{}`",
            request.operation
        )]));
    };
    let mut problems: Vec<String> = request
        .args
        .keys()
        .filter(|k| !op.args.iter().any(|a| a.name == k.as_str()))
        .map(|k| format!("`{}` takes no argument `{k}`", op.name))
        .collect();
    problems.extend(
        op.args
            .iter()
            .filter(|a| a.required && !request.args.contains_key(a.name))
            .map(|a| format!("`{}` needs argument `{}`", op.name, a.name)),
    );
    for a in op.args {
        if let Some(v) = request.args.get(a.name)
            && !a.kind.accepts(v)
        {
            problems.push(format!("argument `{}` must be {}", a.name, a.kind.noun()));
        }
    }
    if problems.is_empty() {
        Ok(op)
    } else {
        Err(Error::Invalid(problems))
    }
}

#[cfg(test)]
mod tests {
    use super::Response;
    use serde_json::json;

    #[test]
    fn a_response_round_trips_its_wire_form() {
        let sent = Response {
            operation: "infra.plan".into(),
            ok: false,
            result: json!({ "plan": "0123", "units": [] }),
            violations: vec!["a violation".into()],
        };
        let text = serde_json::to_string(&sent).expect("serialize");
        let back: Response = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, sent);
    }
}

#[cfg(test)]
mod registry_tests;
