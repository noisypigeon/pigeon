//! Mandates: a task the owner approves once, with the code, so the changes carrying it out go
//! through automatically; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::approvals::matches;

/// The most merges a mandate allows.
pub const MAX_BUDGET: u32 = 10;
/// The merges a mandate allows unless it says.
pub const DEFAULT_BUDGET: u32 = 5;
/// The longest a mandate lasts, in hours.
pub const MAX_HOURS: u32 = 72;
/// How long a mandate lasts unless it says, in hours.
pub const DEFAULT_HOURS: u32 = 24;
/// Where ADRs live; a mandate executes one.
pub const DECISIONS: &str = ".claude/decisions/";

/// Where a mandate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Waiting for the owner's code.
    Proposed,
    /// Granted, within its budget and expiry.
    Active,
    /// Stopped by itself; the owner resumes it or lets it end.
    Paused,
    /// Done, expired, used up, or ended.
    Ended,
}

/// A task approved once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mandate {
    /// Its number.
    pub id: u32,
    /// The ADR it executes, from the repository root.
    pub adr: String,
    /// What the task is.
    pub summary: String,
    /// The paths its changes may touch, one glob each.
    pub scope: Vec<String>,
    /// Whether its changes may deploy.
    pub may_deploy: bool,
    /// The merges it allows.
    pub budget: u32,
    /// The merges made under it.
    #[serde(default)]
    pub used: u32,
    /// How long it lasts once granted, in hours.
    pub hours: u32,
    /// When it expires, once granted, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Where it is.
    pub state: State,
    /// Who granted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_by: Option<String>,
    /// Why it paused or ended, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The changes merged under it.
    #[serde(default)]
    pub merged: Vec<u32>,
}

/// The literal part of `glob` before its first wildcard.
fn literal(glob: &str) -> &str {
    glob.find(['*', '?']).map_or(glob, |i| &glob[..i])
}

impl Mandate {
    /// A new mandate, proposed.
    ///
    /// # Errors
    ///
    /// Why it can't be proposed: no ADR under `.claude/decisions/`, an empty scope, or a budget
    /// or lifetime out of range.
    pub fn propose(
        id: u32,
        adr: &str,
        summary: &str,
        scope: Vec<String>,
        may_deploy: bool,
        budget: Option<u32>,
        hours: Option<u32>,
    ) -> Result<Self, String> {
        let is_adr = adr.starts_with(DECISIONS)
            && std::path::Path::new(adr)
                .extension()
                .is_some_and(|e| e == "md")
            && !adr.contains("..");
        if !is_adr {
            return Err(format!("`adr` must be an ADR under {DECISIONS}"));
        }
        if summary.trim().is_empty() {
            return Err("a mandate needs a summary".into());
        }
        let scope: Vec<String> = scope
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if scope.is_empty() || scope.iter().any(|s| s.starts_with('/') || s.contains("..")) {
            return Err("a mandate's scope is one or more relative globs".into());
        }
        let budget = budget.unwrap_or(DEFAULT_BUDGET);
        if !(1..=MAX_BUDGET).contains(&budget) {
            return Err(format!("`budget` is 1 to {MAX_BUDGET} merges"));
        }
        let hours = hours.unwrap_or(DEFAULT_HOURS);
        if !(1..=MAX_HOURS).contains(&hours) {
            return Err(format!("`hours` is 1 to {MAX_HOURS}"));
        }
        Ok(Self {
            id,
            adr: adr.to_string(),
            summary: summary.trim().to_string(),
            scope,
            may_deploy,
            budget,
            used: 0,
            hours,
            expires: None,
            state: State::Proposed,
            granted_by: None,
            why: None,
            merged: Vec::new(),
        })
    }

    /// Whether a change's `scope` lies inside this mandate's: each glob within one of its own.
    #[must_use]
    pub fn covers(&self, scope: &[String]) -> bool {
        !scope.is_empty()
            && scope.iter().all(|c| {
                let wild = c.contains(['*', '?']);
                self.scope.iter().any(|m| {
                    if wild {
                        m.ends_with("**")
                            && matches(m, literal(c))
                            && literal(c).len() >= literal(m).len()
                    } else {
                        matches(m, c)
                    }
                })
            })
    }

    /// Whether it may approve now: active, unexpired and within its budget.
    #[must_use]
    pub fn usable(&self, now: u64) -> bool {
        self.state == State::Active
            && self.expires.is_some_and(|e| now < e)
            && self.used < self.budget
    }
}

/// Every mandate, and the grants that let the owner approve one on its page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mandates {
    /// The last number given.
    #[serde(default)]
    pub last: u32,
    /// The mandates, oldest first.
    #[serde(default)]
    pub list: Vec<Mandate>,
    /// Open grants by token hash: the mandate and when the grant expires.
    #[serde(default)]
    pub grants: BTreeMap<String, (u32, u64)>,
}

impl Mandates {
    /// Proposes a mandate, numbered next: the next mandates, and it or why it can't be.
    pub fn propose(
        &self,
        adr: &str,
        summary: &str,
        scope: Vec<String>,
        may_deploy: bool,
        budget: Option<u32>,
        hours: Option<u32>,
    ) -> (Self, Result<Mandate, String>) {
        let proposed = Mandate::propose(
            self.last + 1,
            adr,
            summary,
            scope,
            may_deploy,
            budget,
            hours,
        );
        match proposed {
            Ok(m) => {
                let mut next = self.clone();
                next.last = m.id;
                next.list.push(m.clone());
                (next, Ok(m))
            }
            Err(why) => (self.clone(), Err(why)),
        }
    }

    /// Mandate `id`.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<&Mandate> {
        self.list.iter().find(|m| m.id == id)
    }

    /// Where mandate `id` sits in the list.
    fn index(&self, id: u32) -> Option<usize> {
        self.list.iter().position(|m| m.id == id)
    }

    /// Mints a grant under `token`'s hash to grant proposed mandate `id`, until `expires`: the
    /// next mandates, and why not when there is no such mandate waiting to be granted.
    pub fn offer(
        &self,
        id: u32,
        token: &str,
        now: u64,
        expires: u64,
    ) -> (Self, Result<(), String>) {
        match self.get(id) {
            Some(m) if m.state == State::Proposed || m.state == State::Paused => {}
            Some(m) => {
                let why = format!("mandate {id} is {:?}; there is nothing to grant", m.state);
                return (self.clone(), Err(why));
            }
            None => return (self.clone(), Err(format!("no mandate {id}"))),
        }
        let mut next = self.clone();
        next.grants.retain(|_, (_, e)| *e > now);
        next.grants.insert(super::grants::key(token), (id, expires));
        (next, Ok(()))
    }

    /// The mandate `token`'s grant names, without using it.
    ///
    /// # Errors
    ///
    /// When the token names none, or it has expired.
    pub fn peek(&self, token: &str, now: u64) -> Result<&Mandate, String> {
        let (id, expires) = self
            .grants
            .get(&super::grants::key(token))
            .ok_or("this link is not valid; ask for a new one")?;
        if *expires <= now {
            return Err("this link has expired; ask for a new one".into());
        }
        self.get(*id)
            .ok_or_else(|| "its mandate is gone".to_string())
    }

    /// Grants (or resumes) the mandate `token` names, as `by`: active for its hours from `now`.
    /// The grant is spent even when refused, so the next mandates hold it spent either way.
    pub fn grant(&self, token: &str, by: &str, now: u64) -> (Self, Result<Mandate, String>) {
        let id = match self.peek(token, now) {
            Ok(m) => m.id,
            Err(why) => return (self.clone(), Err(why)),
        };
        let mut next = self.clone();
        next.grants.remove(&super::grants::key(token));
        let Some(i) = next.index(id) else {
            return (next, Err("its mandate is gone".into()));
        };
        let m = &mut next.list[i];
        match m.state {
            State::Proposed => m.expires = Some(now + u64::from(m.hours) * 3600),
            State::Paused => {}
            other => {
                let why = format!("mandate {id} is {other:?}");
                return (next, Err(why));
            }
        }
        m.state = State::Active;
        m.granted_by = Some(by.to_string());
        m.why = None;
        let granted = m.clone();
        (next, Ok(granted))
    }

    /// Ends mandate `id`, saying `why`; anyone may, since it only takes power away. The next
    /// mandates, and it or why not when there is no such mandate.
    pub fn end(&self, id: u32, why: &str) -> (Self, Result<Mandate, String>) {
        let Some(i) = self.index(id) else {
            return (self.clone(), Err(format!("no mandate {id}")));
        };
        let mut next = self.clone();
        let m = &mut next.list[i];
        m.state = State::Ended;
        m.why = Some(why.to_string());
        let ended = m.clone();
        (next, Ok(ended))
    }
}

/// What a mandate says about an operation on a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Approve it under the mandate.
    Allow,
    /// Ask the owner, as without a mandate, and say why.
    Ask(String),
    /// Ask the owner, and pause the mandate, saying why.
    Pause(String),
}

/// Whether `m` approves `operation` on `change` at `now`.
/// - It must be usable, cited by the change's plan, and cover the change's scope.
/// - Approving and merging also need the head classified, never by a guarded or sentinel path,
///   and at most confirm, or code only because it deploys and the mandate may deploy.
#[must_use]
pub fn decide(m: &Mandate, change: &crate::changes::Change, operation: &str, now: u64) -> Decision {
    use crate::approvals::Tier;
    if !m.usable(now) {
        return Decision::Ask(format!(
            "mandate {} is {:?}, expired or used up",
            m.id, m.state
        ));
    }
    if !change.plan.contains(&m.adr) {
        return Decision::Ask(format!(
            "change {}'s plan doesn't cite mandate {}'s ADR, {}",
            change.number, m.id, m.adr
        ));
    }
    if !m.covers(&change.scope) {
        return Decision::Pause(format!(
            "change {} reaches outside mandate {}'s scope",
            change.number, m.id
        ));
    }
    if operation == crate::control::BREAK_GLASS {
        return Decision::Ask(format!(
            "`{operation}` on change {} is the owner's each time",
            change.number
        ));
    }
    if !matches!(operation, "change.approve" | "change.merge") {
        return Decision::Allow;
    }
    let Some(tiers) = &change.tiers else {
        return Decision::Ask(format!("change {}'s head has no tiers yet", change.number));
    };
    let merge = &tiers.merge;
    if merge.rule.starts_with("guarded") || merge.rule.starts_with("sentinel") {
        return Decision::Ask(format!(
            "change {} touches a guarded or sentinel path ({}), which always needs the code",
            change.number, merge.rule
        ));
    }
    match merge.tier {
        Tier::Auto | Tier::Confirm => Decision::Allow,
        Tier::Code if merge.rule == "deploys" && m.may_deploy => Decision::Allow,
        Tier::Code if merge.rule == "deploys" => Decision::Ask(format!(
            "change {} deploys, and mandate {} may not",
            change.number, m.id
        )),
        Tier::Code => Decision::Ask(format!(
            "change {} needs the code ({})",
            change.number, merge.rule
        )),
        Tier::Owner => Decision::Ask(format!(
            "change {} needs the owner each time ({})",
            change.number, merge.rule
        )),
    }
}

impl Mandates {
    /// Decides `operation` on `change` under its mandate, recording a merge against the budget
    /// and pausing the mandate when the decision says so: the next mandates, and the mandate and
    /// decision, or none when the change names none.
    #[must_use]
    pub fn decide(
        &self,
        change: &crate::changes::Change,
        operation: &str,
        now: u64,
    ) -> (Self, Option<(u32, Decision)>) {
        let Some(id) = change.mandate else {
            return (self.clone(), None);
        };
        let Some(i) = self.index(id) else {
            return (
                self.clone(),
                Some((id, Decision::Ask(format!("no mandate {id}")))),
            );
        };
        let mut next = self.clone();
        let m = &mut next.list[i];
        let d = decide(m, change, operation, now);
        match &d {
            Decision::Allow if operation == "change.merge" => {
                m.used += 1;
                m.merged.push(change.number);
            }
            Decision::Pause(why) => {
                m.state = State::Paused;
                m.why = Some(why.clone());
            }
            _ => {}
        }
        (next, Some((id, d)))
    }

    /// Pauses every active mandate, saying `why`, as when a deploy fails: the next mandates, and
    /// the ones paused.
    #[must_use]
    pub fn pause_all(&self, why: &str) -> (Self, Vec<u32>) {
        let mut next = self.clone();
        let paused = next
            .list
            .iter_mut()
            .filter(|m| m.state == State::Active)
            .map(|m| {
                m.state = State::Paused;
                m.why = Some(why.to_string());
                m.id
            })
            .collect();
        (next, paused)
    }
}
