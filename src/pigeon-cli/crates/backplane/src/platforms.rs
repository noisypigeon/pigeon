//! Platform runs: `platform.plan` and `platform.apply` of one `pigeon/platform` unit, each
//! granted on its page with the authenticator code, the plan's page also taking the owner's
//! one-time account token, sealed for the run only; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// How long a sealed token, an offer or a stored plan lasts, in seconds.
pub const LIFETIME: u64 = 24 * 60 * 60;

/// Where a run is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Its plan waits for the owner's grant and token.
    #[default]
    Offered,
    /// Its plan job runs.
    Planning,
    /// Planned; its apply waits for the owner's grant.
    Planned,
    /// Its apply job runs.
    Applying,
    /// Applied.
    Applied,
    /// A job failed, or the owner refused.
    Failed,
    /// Its time ran out before it was applied.
    Expired,
}

impl Stage {
    /// Whether nothing more happens to a run here.
    #[must_use]
    pub fn finished(self) -> bool {
        matches!(self, Self::Applied | Self::Failed | Self::Expired)
    }
}

/// One platform run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Its number.
    pub id: u32,
    /// The unit, e.g. `platform/blue`.
    pub unit: String,
    /// Where it is.
    #[serde(default)]
    pub stage: Stage,
    /// The SHA-256 of the pending grant's token, while one is offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<String>,
    /// When the pending grant expires.
    #[serde(default)]
    pub offer_expires: u64,
    /// The owner's token, sealed, between the plan's grant and the apply's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
    /// The stored plan's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The plan's or the apply's changes.
    #[serde(default)]
    pub changes: Vec<Value>,
    /// The commit its jobs ran from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The last job's log, its tail, scrubbed of its credentials by the job.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log: String,
    /// Who granted each step, in order.
    #[serde(default)]
    pub granted_by: Vec<String>,
    /// When it was proposed.
    #[serde(default)]
    pub created: u64,
}

impl Run {
    /// The job it runs now: `plan` or `apply`.
    #[must_use]
    pub fn task(&self) -> Option<&'static str> {
        match self.stage {
            Stage::Planning => Some("plan"),
            Stage::Applying => Some("apply"),
            _ => None,
        }
    }

    /// Its running job's ID.
    #[must_use]
    pub fn job_id(&self) -> Option<String> {
        self.task().map(|t| format!("platform-{}-{t}", self.id))
    }

    /// Its running job's argv.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        match (self.stage, &self.plan) {
            (Stage::Planning, _) => vec!["plan".into(), self.unit.clone()],
            (Stage::Applying, Some(p)) => vec!["apply".into(), self.unit.clone(), p.clone()],
            _ => Vec::new(),
        }
    }

    /// The same run with nothing secret: no sealed token, no offer hash.
    #[must_use]
    pub fn shown(&self) -> Self {
        Self {
            offer: None,
            sealed: None,
            ..self.clone()
        }
    }
}

/// Every platform run of an instance.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Runs {
    /// The last number given.
    #[serde(default)]
    pub last: u32,
    /// The runs, oldest first.
    #[serde(default)]
    pub list: Vec<Run>,
}

/// The hex SHA-256 of a grant token, as a run keeps it.
#[must_use]
pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Whether `unit` is a platform unit ID.
#[must_use]
pub fn is_unit(unit: &str) -> bool {
    unit.split_once('/').is_some_and(|(a, n)| {
        a == "platform"
            && !n.is_empty()
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

impl Runs {
    fn get_mut(&mut self, id: u32) -> Result<&mut Run, String> {
        self.list
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("no platform run {id}"))
    }

    /// The run `id`.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<&Run> {
        self.list.iter().find(|r| r.id == id)
    }

    /// Proposes planning `unit`, offering its grant under `token`; refused while another run
    /// of the unit is open.
    ///
    /// # Errors
    ///
    /// A message for a bad unit or an open run of it.
    pub fn propose(&mut self, unit: &str, token: &str, now: u64) -> Result<Run, String> {
        if !is_unit(unit) {
            return Err(format!(
                "`{unit}` is not a platform unit (`platform/<name>`)"
            ));
        }
        if let Some(open) = self
            .list
            .iter()
            .find(|r| r.unit == unit && !r.stage.finished())
        {
            return Err(format!("platform run {} of {unit} is still open", open.id));
        }
        self.last += 1;
        let run = Run {
            id: self.last,
            unit: unit.to_string(),
            stage: Stage::Offered,
            offer: Some(hash(token)),
            offer_expires: now + LIFETIME,
            created: now,
            ..Run::default()
        };
        self.list.push(run.clone());
        Ok(run)
    }

    /// The run whose pending grant `token` is, at `stage`.
    ///
    /// # Errors
    ///
    /// A message when the token isn't the run's pending grant, or it expired.
    pub fn peek(&self, id: u32, token: &str, stage: Stage, now: u64) -> Result<&Run, String> {
        let run = self
            .get(id)
            .ok_or_else(|| format!("no platform run {id}"))?;
        if run.stage != stage || run.offer.as_deref() != Some(hash(token).as_str()) {
            return Err("this link is not this run's pending grant".into());
        }
        if now >= run.offer_expires {
            return Err("this link has expired".into());
        }
        Ok(run)
    }

    /// Grants the plan of run `id` by `by`, keeping the `sealed` token; the run, now planning.
    ///
    /// # Errors
    ///
    /// As [`Runs::peek`].
    pub fn grant_plan(
        &mut self,
        id: u32,
        token: &str,
        sealed: &str,
        by: &str,
        now: u64,
    ) -> Result<Run, String> {
        self.peek(id, token, Stage::Offered, now)?;
        let run = self.get_mut(id)?;
        run.stage = Stage::Planning;
        run.offer = None;
        run.sealed = Some(sealed.to_string());
        run.granted_by.push(by.to_string());
        Ok(run.clone())
    }

    /// Records run `id`'s plan job: planned, offering the apply under `token`, or failed.
    ///
    /// # Errors
    ///
    /// When there is no such run, or it isn't planning.
    pub fn planned(
        &mut self,
        id: u32,
        result: Result<(String, Vec<Value>), String>,
        token: &str,
        now: u64,
    ) -> Result<Run, String> {
        let run = self.get_mut(id)?;
        if run.stage != Stage::Planning {
            return Err(format!("platform run {id} isn't planning"));
        }
        match result {
            Ok((plan, changes)) => {
                run.stage = Stage::Planned;
                run.plan = Some(plan);
                run.changes = changes;
                run.offer = Some(hash(token));
                run.offer_expires = (run.created + LIFETIME).min(now + LIFETIME);
            }
            Err(why) => fail(run, why),
        }
        Ok(run.clone())
    }

    /// Grants the apply of run `id` by `by`; the run, now applying.
    ///
    /// # Errors
    ///
    /// As [`Runs::peek`].
    pub fn grant_apply(&mut self, id: u32, token: &str, by: &str, now: u64) -> Result<Run, String> {
        self.peek(id, token, Stage::Planned, now)?;
        let run = self.get_mut(id)?;
        run.stage = Stage::Applying;
        run.offer = None;
        run.granted_by.push(by.to_string());
        Ok(run.clone())
    }

    /// Records run `id`'s apply job: applied or failed. Either way its token is erased.
    ///
    /// # Errors
    ///
    /// When there is no such run, or it isn't applying.
    pub fn applied(&mut self, id: u32, result: Result<Vec<Value>, String>) -> Result<Run, String> {
        let run = self.get_mut(id)?;
        if run.stage != Stage::Applying {
            return Err(format!("platform run {id} isn't applying"));
        }
        match result {
            Ok(changes) => {
                run.stage = Stage::Applied;
                run.changes = changes;
                run.sealed = None;
            }
            Err(why) => fail(run, why),
        }
        Ok(run.clone())
    }

    /// Refuses run `id`'s pending grant: failed, its token erased.
    ///
    /// # Errors
    ///
    /// When there is no such run, or nothing is pending.
    pub fn refuse(&mut self, id: u32, by: &str) -> Result<Run, String> {
        let run = self.get_mut(id)?;
        if !matches!(run.stage, Stage::Offered | Stage::Planned) {
            return Err(format!("platform run {id} has nothing pending"));
        }
        fail(run, format!("refused by {by}"));
        Ok(run.clone())
    }

    /// Expires every open run past its lifetime, erasing its token; the runs it expired.
    pub fn expire(&mut self, now: u64) -> Vec<u32> {
        let mut gone = Vec::new();
        for r in &mut self.list {
            let waiting = matches!(r.stage, Stage::Offered | Stage::Planned);
            let late = now >= r.created + LIFETIME || (waiting && now >= r.offer_expires);
            if !r.stage.finished() && late && r.task().is_none() {
                r.stage = Stage::Expired;
                r.offer = None;
                r.sealed = None;
                gone.push(r.id);
            }
        }
        gone
    }

    /// The runs whose jobs are running.
    #[must_use]
    pub fn running(&self) -> Vec<&Run> {
        self.list.iter().filter(|r| r.task().is_some()).collect()
    }
}

fn fail(run: &mut Run, why: String) {
    run.stage = Stage::Failed;
    run.why = Some(why);
    run.offer = None;
    run.sealed = None;
}
