//! Approval tiers: which of auto, confirm and code a change's operations need, from the rules
//! in `.config/approvals.yaml` on `main`; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md`.

use serde::{Deserialize, Serialize};

/// Where the rules live, read from the base branch, never the change's.
pub const RULES: &str = ".config/approvals.yaml";

/// Who approves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// The control plane, under a rule or a mandate.
    Auto,
    /// The owner, with one tap.
    Confirm,
    /// The owner, with the authenticator code.
    Code,
    /// The owner, proven, for each operation: destruction and secrets; see
    /// `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §2.
    Owner,
}

/// A tier and the rule that chose it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tiered {
    /// The tier.
    pub tier: Tier,
    /// The rule, e.g. `guarded: .mcp.json`.
    pub rule: String,
}

/// The tiers of a change's operations, for one head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tiers {
    /// Authorizing its plan, from its scope.
    pub authorize: Tiered,
    /// Approving and merging its head, from its files.
    pub merge: Tiered,
    /// Closing it unmerged.
    pub close: Tiered,
}

/// The rules: path globs, `*` within a segment and `**` across them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// Always code, and never in a mandate.
    pub guarded: Vec<String>,
    /// Code without a mandate that may deploy; a Worker's inputs count too.
    #[serde(default)]
    pub deploying: Vec<String>,
    /// Spikes: authorized and closed automatically, never merged automatically.
    #[serde(default)]
    pub spikes: Vec<String>,
    /// Where ADRs live; a change that only appends to existing ones is automatic.
    #[serde(default)]
    pub decisions: Option<String>,
    /// Paths whose change adds, reads or exposes a secret: always the owner.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Files whose `secrets:` list a change may lengthen, which needs the owner.
    #[serde(default)]
    pub secret_lists: Vec<String>,
}

/// Whether `path` matches `glob`.
#[must_use]
pub fn matches(glob: &str, path: &str) -> bool {
    fn go(g: &[u8], p: &[u8]) -> bool {
        match g {
            [] => p.is_empty(),
            [b'*', b'*', b'/', rest @ ..] => {
                go(rest, p)
                    || p.iter()
                        .enumerate()
                        .any(|(i, c)| *c == b'/' && go(rest, &p[i + 1..]))
            }
            [b'*', b'*'] => true,
            [b'*', rest @ ..] => (0..=p.len())
                .take_while(|&i| i == 0 || p[i - 1] != b'/')
                .any(|i| go(rest, &p[i..])),
            [b'?', rest @ ..] => p.first().is_some_and(|c| *c != b'/') && go(rest, &p[1..]),
            [c, rest @ ..] => p.first() == Some(c) && go(rest, &p[1..]),
        }
    }
    go(glob.as_bytes(), path.as_bytes())
}

/// The literal part of `glob` before its first wildcard.
fn literal(glob: &str) -> &str {
    glob.find(['*', '?']).map_or(glob, |i| &glob[..i])
}

/// Whether a scope glob may reach a path `pattern` matches: one's literal prefix starts the
/// other. A pattern with no literal prefix, such as `**/x`, counts only where it matches the scope
/// itself; the merge's tier checks the files it really touches.
#[must_use]
pub fn overlaps(scope: &str, pattern: &str) -> bool {
    let (a, b) = (literal(scope), literal(pattern));
    if b.is_empty() {
        return matches(pattern, scope);
    }
    a.starts_with(b) || b.starts_with(a)
}

fn tiered(tier: Tier, rule: impl Into<String>) -> Tiered {
    Tiered {
        tier,
        rule: rule.into(),
    }
}

/// What a head's files are, beyond their paths.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// Its merge deploys a Worker (`cobot::scope::workers`).
    pub deploys_worker: bool,
    /// Every file under `decisions` existed on the base and only gained lines.
    pub appends_only: bool,
    /// A `secret_lists` file names a secret the base's doesn't.
    pub adds_secret: bool,
}

/// The tiers of a change with `scope` whose head touches `files`.
#[must_use]
pub fn classify(rules: &Rules, scope: &[String], files: &[String], facts: Facts) -> Tiers {
    let secret_file = files
        .iter()
        .find(|f| rules.secrets.iter().any(|g| matches(g, f)));
    let secret_scope = scope
        .iter()
        .find(|s| rules.secrets.iter().any(|g| overlaps(s, g)));
    let guarded_file = files
        .iter()
        .find(|f| rules.guarded.iter().any(|g| matches(g, f)));
    let guarded_scope = scope
        .iter()
        .find(|s| rules.guarded.iter().any(|g| overlaps(s, g)));
    let spike = |paths: &[String], m: &dyn Fn(&str, &str) -> bool| {
        !paths.is_empty() && paths.iter().all(|p| rules.spikes.iter().any(|g| m(g, p)))
    };
    let decisions = rules.decisions.as_deref().unwrap_or("\u{0}");
    let adr_only =
        !files.is_empty() && facts.appends_only && files.iter().all(|f| f.starts_with(decisions));
    let authorize = if let Some(s) = secret_scope {
        tiered(Tier::Owner, format!("secrets: {s}"))
    } else if let Some(s) = guarded_scope {
        tiered(Tier::Code, format!("guarded: {s}"))
    } else if spike(scope, &|g, s| g == s || matches(g, literal(s))) {
        tiered(Tier::Auto, "spike")
    } else if !scope.is_empty() && scope.iter().all(|s| s.starts_with(decisions)) {
        tiered(Tier::Confirm, "decisions")
    } else if scope
        .iter()
        .any(|s| rules.deploying.iter().any(|d| overlaps(s, d)))
    {
        tiered(Tier::Code, "deploys")
    } else {
        tiered(Tier::Confirm, "routine")
    };
    let merge = if let Some(f) = secret_file {
        tiered(Tier::Owner, format!("secrets: {f}"))
    } else if facts.adds_secret {
        tiered(Tier::Owner, "adds a secret")
    } else if let Some(f) = guarded_file {
        tiered(Tier::Code, format!("guarded: {f}"))
    } else if spike(files, &|g, f| matches(g, f)) {
        tiered(Tier::Confirm, "spike, merged")
    } else if adr_only {
        tiered(Tier::Auto, "ADR amendment")
    } else if facts.deploys_worker
        || files
            .iter()
            .any(|f| rules.deploying.iter().any(|d| matches(d, f)))
    {
        tiered(Tier::Code, "deploys")
    } else {
        tiered(Tier::Confirm, "routine")
    };
    let close = if spike(files, &|g, f| matches(g, f)) {
        tiered(Tier::Auto, "spike")
    } else {
        tiered(Tier::Confirm, "close")
    };
    Tiers {
        authorize,
        merge,
        close,
    }
}

/// Whether `head` names a secret `before` doesn't.
#[must_use]
pub fn lengthens(before: &[String], head: &[String]) -> bool {
    head.iter().any(|h| !before.contains(h))
}

/// The rule under which the control plane itself approves `operation` on `change`: approving
/// or merging a head whose merge tier is auto or confirm; see
/// `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §1.
#[must_use]
pub fn by_policy(change: &crate::changes::Change, operation: &str) -> Option<String> {
    if operation == "change.close" {
        // Closing loses nothing until something was pushed.
        return change.head.is_none().then(|| "unpushed".to_string());
    }
    if !matches!(operation, "change.approve" | "change.merge") {
        return None;
    }
    let merge = &change.tiers.as_ref()?.merge;
    (merge.tier <= Tier::Confirm).then(|| merge.rule.clone())
}

/// Who approves `operation` on `change` through its change grant at `now`: authorizing,
/// approving or merging at no more than the code tier, while the grant lasts and, to merge,
/// unless the owner kept the merge; see
/// `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §3.
#[must_use]
pub fn by_grant(change: &crate::changes::Change, operation: &str, now: u64) -> Option<String> {
    let g = change.grant.as_ref()?;
    let stage = matches!(
        operation,
        "change.authorize" | "change.approve" | "change.merge"
    );
    let kept = g.review && operation == "change.merge";
    (stage && !kept && now < g.expires && tier_of(change, operation) <= Tier::Code)
        .then(|| g.by.clone())
}

/// The tier `operation` on `change` needs now, from its head's tiers.
/// - Authorizing stays code until the Worker classifies a scope.
/// - Auto counts as confirm until automatic approval lands.
/// - No tiers means code, except to close, which takes nothing from `main`.
#[must_use]
pub fn tier_of(change: &crate::changes::Change, operation: &str) -> Tier {
    let tiers = change.tiers.as_ref();
    let at_least_confirm = |t: &Tiered| t.tier.max(Tier::Confirm);
    match operation {
        "change.approve" | "change.merge" => {
            tiers.map_or(Tier::Code, |t| at_least_confirm(&t.merge))
        }
        // Closing a pushed change throws its work away; see the 2026-11-09 approvals ADR §2.
        "change.close" if change.head.is_some() => Tier::Owner,
        "change.close" => tiers.map_or(Tier::Confirm, |t| at_least_confirm(&t.close)),
        _ => Tier::Code,
    }
}
