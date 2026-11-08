//! The operation catalog and its wire form, shared by the `engines` and `worker` builds
//! (control-plane ADR §1).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Error;

/// Who runs an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Backplane itself.
    Backplane,
    /// Policy and its enforcement.
    Sentinel,
    /// Actions: infrastructure, releases.
    Cobot,
    /// Planned: scheduling and data operations.
    Gantry,
}

impl Engine {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Backplane => "backplane",
            Self::Sentinel => "sentinel",
            Self::Cobot => "cobot",
            Self::Gantry => "gantry",
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
}

impl Kind {
    fn accepts(self, v: &Value) -> bool {
        match self {
            Self::String => v.is_string(),
            Self::Text => v.is_string() || v.is_number() || v.is_boolean(),
            Self::Strings => v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::String => "a string",
            Self::Text => "text",
            Self::Strings => "a list of strings",
        }
    }

    /// Its JSON Schema.
    #[must_use]
    pub fn schema(self) -> Value {
        match self {
            Self::String => json!({ "type": "string" }),
            Self::Text => json!({ "type": ["string", "number", "boolean"] }),
            Self::Strings => json!({ "type": "array", "items": { "type": "string" } }),
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

const fn arg(name: &'static str, required: bool, kind: Kind) -> Arg {
    Arg {
        name,
        required,
        kind,
    }
}

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

    /// Whether only the control plane runs it, from its Durable Objects and D1.
    #[must_use]
    pub fn control_plane(&self) -> bool {
        self.name.starts_with("change.")
            || self.name.starts_with("ci.")
            || self.name.starts_with("deploy.")
            || self.name.starts_with("git.")
            || self.name.starts_with("mandate.")
            || self.name.starts_with("task.")
    }
}

const PR_FIELDS: &[Arg] = &[
    arg("title", false, Kind::Text),
    arg("body", false, Kind::Text),
    arg("number", false, Kind::Text),
    arg("files", false, Kind::Text),
    arg("branch", false, Kind::Text),
    arg("scope", false, Kind::Text),
];

const NUMBER: Arg = arg("number", true, Kind::Text);

const CATALOG: &[Operation] = &[
    Operation {
        name: "instance.show",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Show the instance manifest",
        args: &[],
    },
    Operation {
        name: "architecture.describe",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Describe the instance's architecture as a graph and its layout",
        args: &[],
    },
    Operation {
        name: "policy.check",
        engine: Engine::Sentinel,
        effect: Effect::Read,
        summary: "Check the instance against its policies (args: title?, body?, number?, files?, which add the pull-request policy; branch? and scope? add the change policy)",
        args: PR_FIELDS,
    },
    Operation {
        name: "infra.validate",
        engine: Engine::Cobot,
        effect: Effect::Read,
        summary: "Validate every infrastructure and workload unit",
        args: &[],
    },
    Operation {
        name: "change.scope",
        engine: Engine::Cobot,
        effect: Effect::Read,
        summary: "The checks a change needs, from the paths it touches, in the order to run them (args: files, one path per line)",
        args: &[arg("files", true, Kind::Text)],
    },
    Operation {
        name: "change.propose",
        engine: Engine::Backplane,
        effect: Effect::Plan,
        summary: "Propose a change: its slug, Conventional Commit title, plan and scope (globs, one per line), and optionally the `mandate` it is carried out under, the `session` proposing it, and for a design change the globs its ADR `affects`; returns its number, its branch, change/<number>-<slug>, and the open changes it `overlaps`",
        args: &[
            arg("slug", true, Kind::String),
            arg("title", true, Kind::String),
            arg("plan", true, Kind::String),
            arg("scope", true, Kind::String),
            arg("mandate", false, Kind::Text),
            arg("session", false, Kind::String),
            arg("affects", false, Kind::String),
        ],
    },
    Operation {
        name: "change.authorize",
        engine: Engine::Backplane,
        effect: Effect::Apply,
        summary: "Approve a proposed change's plan, so its branch may be pushed; never by its proposer",
        args: &[NUMBER],
    },
    Operation {
        name: "change.pushed",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "Report a push of a change's branch, naming its head, when the push event may be missed; the bound job refuses a SHA that isn't the branch's head",
        args: &[NUMBER, arg("sha", true, Kind::String)],
    },
    Operation {
        name: "change.status",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "A change's state, head, approvals, violations and each job's result",
        args: &[NUMBER],
    },
    Operation {
        name: "change.list",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Every change, newest first (args: state?)",
        args: &[arg("state", false, Kind::String)],
    },
    Operation {
        name: "change.approve",
        engine: Engine::Backplane,
        effect: Effect::Apply,
        summary: "Approve a checked change's head for merge (sha?: the head; a person's call without it approves the current head, and a service token's call asks the owner for their code in the card); never by its proposer",
        args: &[NUMBER, arg("sha", false, Kind::String)],
    },
    Operation {
        name: "change.merge",
        engine: Engine::Backplane,
        effect: Effect::Apply,
        summary: "Merge an approved change: its head is squashed onto main as `<title> (!<number>)` and its branch deleted; main must be an ancestor of the head",
        args: &[NUMBER],
    },
    Operation {
        name: "change.close",
        engine: Engine::Backplane,
        effect: Effect::Apply,
        summary: "Close a change unmerged, stopping its jobs",
        args: &[NUMBER],
    },
    Operation {
        name: "ci.rerun",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "Run one of a finished head's jobs again, and those its failure cancelled",
        args: &[NUMBER, arg("job", true, Kind::String)],
    },
    Operation {
        name: "git.tokens",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "The repository's git tokens: each one's ID, scope, state and expiry, never its value",
        args: &[],
    },
    Operation {
        name: "sites.build",
        engine: Engine::Cobot,
        effect: Effect::Build,
        summary: "Build static sites into the work directory with the instance's architecture as data; publishes nothing (args: sites?)",
        args: &[arg("sites", false, Kind::Strings)],
    },
    Operation {
        name: "sites.publish",
        engine: Engine::Cobot,
        effect: Effect::Apply,
        summary: "Build the named sites, or every site, with the architecture as data, sync each to its bucket and check it live, with the run's CLOUDFLARE_API_TOKEN (args: sites?)",
        args: &[arg("sites", false, Kind::Strings)],
    },
    Operation {
        name: "deploy.status",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "What main's pipeline last published: the SHA deployed, the one in progress, and recent deploys with their logs",
        args: &[],
    },
    Operation {
        name: "mandate.propose",
        engine: Engine::Backplane,
        effect: Effect::Plan,
        summary: "Propose a mandate: a task the owner grants once with the code, so changes inside it go through without asking; `adr` (the ADR it executes), `summary`, `scope` (globs, one per line), `may_deploy`, `budget` (merges, 1 to 10, default 5) and `hours` (1 to 72, default 24); answers with the page's link",
        args: &[
            arg("adr", true, Kind::String),
            arg("summary", true, Kind::String),
            arg("scope", true, Kind::String),
            arg("may_deploy", false, Kind::Text),
            arg("budget", false, Kind::Text),
            arg("hours", false, Kind::Text),
        ],
    },
    Operation {
        name: "task.list",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "The open tasks `session` may take: its own at once, any after 30 minutes unclaimed, such as a reconciliation to carry out",
        args: &[arg("session", false, Kind::String)],
    },
    Operation {
        name: "task.wait",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Waits up to `seconds` (at most 50) for an unclaimed task `session` may take, then lists them",
        args: &[
            arg("session", false, Kind::String),
            arg("seconds", false, Kind::Text),
        ],
    },
    Operation {
        name: "task.claim",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "Take task `id` for `session`, so no other session works on it",
        args: &[
            arg("id", true, Kind::Text),
            arg("session", true, Kind::String),
        ],
    },
    Operation {
        name: "task.done",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "Mark task `id` done; a task on a change that merges is done by itself",
        args: &[arg("id", true, Kind::Text)],
    },
    Operation {
        name: "mandate.status",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Every mandate: its scope, budget used, expiry, state and the changes merged under it",
        args: &[],
    },
    Operation {
        name: "mandate.end",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "End mandate `id`, saying `why`; open to anyone, since it only takes power away",
        args: &[arg("id", true, Kind::Text), arg("why", false, Kind::String)],
    },
    Operation {
        name: "approval.wait",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "Show the owner's code card for a pending approval of `operation` (change.authorize, change.approve, change.merge or change.close) on change `number`, on a client that shows forms; the page's link works meanwhile",
        args: &[NUMBER, arg("operation", true, Kind::String)],
    },
    Operation {
        name: "deploy.retry",
        engine: Engine::Backplane,
        effect: Effect::Build,
        summary: "Deploy main's newest push again: after a failed deploy, as when its image was missing, or after a passed one, everything, as when new images must roll out",
        args: &[],
    },
    Operation {
        name: "infra.plan",
        engine: Engine::Cobot,
        effect: Effect::Plan,
        summary: "Plan units and store the plan for 24 hours (args: units?; state, which serve supplies)",
        args: &[
            arg("state", true, Kind::String),
            arg("units", false, Kind::Strings),
        ],
    },
    Operation {
        name: "infra.apply",
        engine: Engine::Cobot,
        effect: Effect::Apply,
        summary: "Apply a stored, unexpired plan exactly (args: plan; state, which serve supplies)",
        args: &[
            arg("state", true, Kind::String),
            arg("plan", true, Kind::String),
        ],
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
