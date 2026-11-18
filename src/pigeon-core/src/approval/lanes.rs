//! The owner's fast lane: for a short window, a named line of work's code checks are waived and
//! its changes go to the front of the merge queue; see
//! `.claude/decisions/2026-11-17-security-owner-fast-lane-window.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::approval::tiers::Tier;
use crate::change::Change;

#[cfg(test)]
#[path = "lanes_tests.rs"]
mod tests;

/// How long a lane lasts unless it says, in minutes.
pub const DEFAULT_MINUTES: u32 = 30;
/// The longest a lane lasts from its grant, in minutes.
pub const MAX_MINUTES: u32 = 120;
/// How many lanes may be open at once.
pub const MAX_OPEN: usize = 1;
/// How long a request's link lasts, in seconds.
pub const OFFER_SECONDS: u64 = 3_600;

/// The jobs a lane never waives: the bounds, scope, policies and tiers, infrastructure's
/// validation, the restack, the mirror and the merge. See the ADR's §3.
pub const NEVER_WAIVED: &[&str] = &[
    "bound",
    "policy.check",
    "infra.validate",
    "restack",
    "mirror",
    "merge",
];

/// The prefix of a merge gate's job; the job it repeats follows it.
const GATE: &str = "gate: ";

/// Whether a lane may waive the job `name`: a code check, or its merge-gate job, outside
/// [`NEVER_WAIVED`].
#[must_use]
pub fn waivable(name: &str) -> bool {
    let job = name.strip_prefix(GATE).unwrap_or(name).trim();
    !job.is_empty() && !NEVER_WAIVED.contains(&job)
}

/// The line of work a lane names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Line {
    /// Every change a session proposed, by its URL.
    Session(String),
    /// These changes, by number.
    Changes(Vec<u32>),
    /// Every change whose scope lies inside these globs.
    Paths(Vec<String>),
}

impl Line {
    /// The line, checked: a session URL, at least one change, or relative globs.
    ///
    /// # Errors
    ///
    /// Why it names no line of work.
    pub fn checked(self) -> Result<Self, String> {
        match self {
            Self::Session(s) if s.trim().is_empty() => Err("a lane's session is its URL".into()),
            Self::Session(s) => Ok(Self::Session(s.trim().to_string())),
            Self::Changes(mut n) => {
                n.sort_unstable();
                n.dedup();
                if n.is_empty() {
                    Err("a lane names at least one change".into())
                } else {
                    Ok(Self::Changes(n))
                }
            }
            Self::Paths(globs) => {
                let globs: Vec<String> = globs
                    .into_iter()
                    .map(|g| g.trim().to_string())
                    .filter(|g| !g.is_empty())
                    .collect();
                if globs.is_empty() || globs.iter().any(|g| g.starts_with('/') || g.contains(".."))
                {
                    Err("a lane's paths are one or more relative globs".into())
                } else {
                    Ok(Self::Paths(globs))
                }
            }
        }
    }

    /// Whether it names `change`.
    #[must_use]
    pub fn names(&self, change: &Change) -> bool {
        match self {
            Self::Session(s) => change.session.as_deref() == Some(s.as_str()),
            Self::Changes(n) => n.contains(&change.number),
            Self::Paths(globs) => within(globs, &change.scope),
        }
    }

    /// This line with `more` added: the same kind only.
    ///
    /// # Errors
    ///
    /// When `more` is of another kind, or names another session.
    pub fn widened(&self, more: &Self) -> Result<Self, String> {
        match (self, more) {
            (Self::Session(a), Self::Session(b)) if a == b => Ok(self.clone()),
            (Self::Changes(a), Self::Changes(b)) => {
                Self::Changes(a.iter().chain(b).copied().collect()).checked()
            }
            (Self::Paths(a), Self::Paths(b)) => {
                let mut all = a.clone();
                all.extend(b.iter().filter(|g| !a.contains(g)).cloned());
                Ok(Self::Paths(all))
            }
            _ => Err("a lane widens only with more of the same kind of line".into()),
        }
    }
}

/// Whether each of `scope`'s globs lies inside one of `globs`, by the mandate's rule.
fn within(globs: &[String], scope: &[String]) -> bool {
    let probe = crate::approval::mandates::Mandate {
        id: 0,
        adr: String::new(),
        summary: String::new(),
        scope: globs.to_vec(),
        may_deploy: false,
        budget: 0,
        used: 0,
        hours: 0,
        expires: None,
        state: crate::approval::mandates::State::Proposed,
        granted_by: None,
        why: None,
        merged: Vec::new(),
        hold: None,
    };
    probe.covers(scope)
}

/// Where a lane is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Asked for, waiting for the owner's passkey.
    Offered,
    /// Granted, until its end.
    Open,
    /// Past its end, ended, or refused.
    Ended,
}

/// A waived job: the change, its head and the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waived {
    /// The change.
    pub change: u32,
    /// The head it was waived on.
    pub head: String,
    /// The job.
    pub job: String,
}

/// A fast lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    /// Its number.
    pub id: u32,
    /// Why the owner wants it.
    pub reason: String,
    /// The line of work it names.
    pub line: Line,
    /// How long it lasts from its grant.
    pub minutes: u32,
    /// Who asked for it.
    pub asked_by: String,
    /// The open lane a request widens or extends, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<u32>,
    /// When it was granted, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
    /// When it ends, once granted, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    /// Who granted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Where it is.
    pub state: State,
    /// Why it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Every job waived under it.
    #[serde(default)]
    pub waived: Vec<Waived>,
    /// The changes merged under it.
    #[serde(default)]
    pub merged: Vec<u32>,
}

impl Lane {
    /// Whether it is open at `now`; one past its end is closed without anyone closing it.
    #[must_use]
    pub fn open(&self, now: u64) -> bool {
        self.state == State::Open && self.until.is_some_and(|u| now < u)
    }
}

/// Whether `lane` covers `change`'s current head at `now`; why not, when it doesn't. See the
/// ADR's §2.
///
/// # Errors
///
/// Why the change is outside the lane.
pub fn covers(lane: &Lane, change: &Change, now: u64) -> Result<(), String> {
    let n = change.number;
    if !lane.open(now) {
        return Err(format!("lane {} isn't open", lane.id));
    }
    if !lane.line.names(change) {
        return Err(format!("lane {} doesn't name change {n}", lane.id));
    }
    let Some(tiers) = &change.tiers else {
        return Err(format!("change {n}'s head has no tiers yet"));
    };
    let merge = &tiers.merge;
    if merge.tier == Tier::Owner
        || merge.rule.starts_with("guarded")
        || crate::approval::tiers::is_judge_rule(&merge.rule)
    {
        return Err(format!(
            "change {n} touches an owner, guarded or eviction path ({}), which no lane covers",
            merge.rule
        ));
    }
    if change.awaits_review() || change.escalated.is_some() {
        return Err(format!(
            "change {n}'s head waits for its planner, which no lane skips"
        ));
    }
    Ok(())
}

/// Every lane, and the grants that let the owner open one on its page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lanes {
    /// The last number given.
    #[serde(default)]
    pub last: u32,
    /// The lanes, oldest first.
    #[serde(default)]
    pub list: Vec<Lane>,
    /// Open grants by token hash: the lane and when the grant expires.
    #[serde(default)]
    pub grants: BTreeMap<String, (u32, u64)>,
}

impl Lanes {
    /// Lane `id`.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<&Lane> {
        self.list.iter().find(|l| l.id == id)
    }

    fn index(&self, id: u32) -> Option<usize> {
        self.list.iter().position(|l| l.id == id)
    }

    /// The lane open at `now`, if any.
    #[must_use]
    pub fn current(&self, now: u64) -> Option<&Lane> {
        self.list.iter().find(|l| l.open(now))
    }

    /// Asks for a lane, or to widen or extend open lane `extends`, under `token` until the
    /// link's expiry: the next lanes, and it or why it can't be. Asking opens nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &self,
        line: Line,
        minutes: Option<u32>,
        reason: &str,
        asked_by: &str,
        extends: Option<u32>,
        token: &str,
        now: u64,
    ) -> (Self, Result<Lane, String>) {
        let asked = (|| {
            let line = line.checked()?;
            if reason.trim().is_empty() {
                return Err("a lane needs a reason".to_string());
            }
            let minutes = minutes.unwrap_or(DEFAULT_MINUTES);
            if !(1..=MAX_MINUTES).contains(&minutes) {
                return Err(format!("`minutes` is 1 to {MAX_MINUTES}"));
            }
            if let Some(id) = extends {
                let open = self.get(id).is_some_and(|l| l.open(now));
                if !open {
                    return Err(format!("lane {id} isn't open; ask for a new lane"));
                }
            }
            Ok((line, minutes))
        })();
        let (line, minutes) = match asked {
            Ok(a) => a,
            Err(why) => return (self.clone(), Err(why)),
        };
        let mut next = self.clone();
        next.last += 1;
        let offered = Lane {
            id: next.last,
            reason: reason.trim().to_string(),
            line,
            minutes,
            asked_by: asked_by.to_string(),
            extends,
            started: None,
            until: None,
            by: None,
            state: State::Offered,
            why: None,
            waived: Vec::new(),
            merged: Vec::new(),
        };
        next.list.push(offered.clone());
        next.grants.retain(|_, (_, e)| *e > now);
        next.grants
            .insert(super::grants::key(token), (offered.id, now + OFFER_SECONDS));
        (next, Ok(offered))
    }

    /// The lane `token`'s grant names, without using it.
    ///
    /// # Errors
    ///
    /// When the token names none, or it has expired.
    pub fn peek(&self, token: &str, now: u64) -> Result<&Lane, String> {
        let (id, expires) = self
            .grants
            .get(&super::grants::key(token))
            .ok_or("this link is not valid; ask for a new one")?;
        if *expires <= now {
            return Err("this link has expired; ask for a new one".into());
        }
        self.get(*id).ok_or_else(|| "its lane is gone".to_string())
    }

    /// Grants the request `token` names, as the owner `by`, proven by passkey: it opens for its
    /// minutes from `now`, or widens and extends the lane it names, to at most [`MAX_MINUTES`]
    /// from `now`. The grant is spent even when refused.
    pub fn grant(&self, token: &str, by: &str, now: u64) -> (Self, Result<Lane, String>) {
        let id = match self.peek(token, now) {
            Ok(l) => l.id,
            Err(why) => return (self.clone(), Err(why)),
        };
        let mut next = self.clone();
        next.grants.remove(&super::grants::key(token));
        let Some(i) = next.index(id) else {
            return (next, Err("its lane is gone".into()));
        };
        if next.list[i].state != State::Offered {
            let why = format!("lane {id} is {:?}", next.list[i].state);
            return (next, Err(why));
        }
        let until = now + u64::from(next.list[i].minutes) * 60;
        match next.list[i].extends {
            Some(open) => next.extend(i, open, by, until, now),
            None => next.open(i, by, until, now),
        }
    }

    /// Opens offered lane `i` as `by` until `until`, unless [`MAX_OPEN`] are open.
    fn open(mut self, i: usize, by: &str, until: u64, now: u64) -> (Self, Result<Lane, String>) {
        let open = self.list.iter().filter(|l| l.open(now)).count();
        let mut l = self.list[i].clone();
        if open >= MAX_OPEN {
            (l.state, l.why) = (State::Ended, Some("another lane is open".into()));
            self.list[i] = l;
            let why = format!("{MAX_OPEN} lane is open; widen or extend it instead");
            return (self, Err(why));
        }
        (l.state, l.started, l.until, l.by) =
            (State::Open, Some(now), Some(until), Some(by.into()));
        self.list[i] = l.clone();
        (self, Ok(l))
    }

    /// Grants request `i` into open lane `open`: its line widened, its end moved to `until`.
    fn extend(
        mut self,
        i: usize,
        open: u32,
        by: &str,
        until: u64,
        now: u64,
    ) -> (Self, Result<Lane, String>) {
        let request = self.list[i].clone();
        self.list[i].state = State::Ended;
        let Some(o) = self.index(open).filter(|&o| self.list[o].open(now)) else {
            self.list[i].why = Some(format!("lane {open} closed first"));
            return (
                self,
                Err(format!("lane {open} isn't open; ask for a new lane")),
            );
        };
        let wider = match self.list[o].line.widened(&request.line) {
            Ok(w) => w,
            Err(why) => {
                self.list[i].why = Some(why.clone());
                return (self, Err(why));
            }
        };
        self.list[i].why = Some(format!("granted into lane {open}"));
        let mut lane = self.list[o].clone();
        lane.line = wider;
        lane.until = lane.until.map(|u| u.max(until));
        lane.by = Some(by.into());
        self.list[o] = lane.clone();
        (self, Ok(lane))
    }

    /// Ends lane `id`, saying `why`; open to the owner and the sessions it names, since it only
    /// takes power away.
    pub fn end(&self, id: u32, why: &str) -> (Self, Result<Lane, String>) {
        let Some(i) = self.index(id) else {
            return (self.clone(), Err(format!("no lane {id}")));
        };
        if self.list[i].state == State::Ended {
            return (self.clone(), Err(format!("lane {id} has ended")));
        }
        let mut next = self.clone();
        let l = &mut next.list[i];
        (l.state, l.why) = (State::Ended, Some(why.to_string()));
        let ended = l.clone();
        (next, Ok(ended))
    }

    /// Ends every lane past its end at `now`: the next lanes, and those it ended.
    #[must_use]
    pub fn settle(&self, now: u64) -> (Self, Vec<Lane>) {
        let mut next = self.clone();
        let mut ended = Vec::new();
        for l in &mut next.list {
            if l.state == State::Open && !l.open(now) {
                (l.state, l.why) = (State::Ended, Some("expired".into()));
                ended.push(l.clone());
            }
        }
        next.grants.retain(|_, (_, e)| *e > now);
        (next, ended)
    }

    /// Ends the open lane, if any, saying `why`: a failed deploy, a failed deferred run, or Auto
    /// paused.
    #[must_use]
    pub fn end_open(&self, why: &str, now: u64) -> (Self, Option<Lane>) {
        match self.current(now).map(|l| l.id) {
            Some(id) => {
                let (next, ended) = self.end(id, why);
                (next, ended.ok())
            }
            None => (self.clone(), None),
        }
    }

    /// Records job `job` waived on `change`'s `head` under the open lane.
    ///
    /// # Errors
    ///
    /// When no lane covers the change, or the job is never waived.
    pub fn waive(&self, change: &Change, job: &str, now: u64) -> Result<(Self, u32), String> {
        if !waivable(job) {
            return Err(format!("`{job}` is never waived"));
        }
        let lane = self
            .current(now)
            .ok_or_else(|| "no lane is open".to_string())?;
        covers(lane, change, now)?;
        let head = change
            .head
            .clone()
            .ok_or_else(|| format!("change {} has no head", change.number))?;
        let id = lane.id;
        let mut next = self.clone();
        if let Some(i) = next.index(id) {
            let w = Waived {
                change: change.number,
                head,
                job: job.to_string(),
            };
            if !next.list[i].waived.contains(&w) {
                next.list[i].waived.push(w);
            }
        }
        Ok((next, id))
    }

    /// Records `change` merged under lane `id`.
    #[must_use]
    pub fn merged(&self, id: u32, change: u32) -> Self {
        let mut next = self.clone();
        if let Some(i) = next.index(id)
            && !next.list[i].merged.contains(&change)
        {
            next.list[i].merged.push(change);
        }
        next
    }
}
