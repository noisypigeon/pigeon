//! D1's lease, naming the acting color, and the fence that fails a batch written under a stale epoch; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §A.3.

use serde::{Deserialize, Serialize};
use serde_json::json;

pub mod step;

/// The lease: who acts, under which epoch, in which state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// The Worker that acts.
    pub color: String,
    /// Rises by one at each flip.
    pub epoch: u64,
    /// serving, draining or promoting.
    pub state: String,
}

/// The color before blue and green exist.
pub const FIRST_COLOR: &str = "pigeon-control";

/// Reads the lease.
#[must_use]
pub fn read() -> crate::control::Statement {
    crate::control::Statement {
        sql: "SELECT color, epoch, state FROM lease WHERE id = 1",
        params: vec![],
    }
}

/// The lease from `read`'s row.
#[must_use]
pub fn from_row(row: &serde_json::Value) -> Option<Lease> {
    Some(Lease {
        color: row.get("color")?.as_str()?.to_string(),
        epoch: row.get("epoch")?.as_u64()?,
        state: row.get("state")?.as_str()?.to_string(),
    })
}

/// The first statement of an act's batch: it fails the batch unless `epoch` is the lease's.
#[must_use]
pub fn fence(epoch: u64) -> crate::control::Statement {
    crate::control::Statement {
        sql: "INSERT OR REPLACE INTO fence (id, ok) VALUES (1, (SELECT CASE WHEN epoch = ?1 THEN 1 END FROM lease WHERE id = 1))",
        params: vec![json!(epoch)],
    }
}

/// Whether `color` acts under `lease`: it holds it, or no lease could be read; fail open until step 9 of .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md.
#[must_use]
pub fn acting(lease: Option<&Lease>, color: &str) -> bool {
    lease.is_none_or(|l| l.color == color)
}

/// Writes `lease` only while the lease's epoch is still `old_epoch`; the one fenced statement a promote's flip is.
#[must_use]
pub fn write(old_epoch: u64, lease: &Lease) -> crate::control::Statement {
    crate::control::Statement {
        sql: "UPDATE lease SET color = ?1, epoch = ?2, state = ?3 WHERE id = 1 AND epoch = ?4",
        params: vec![
            json!(lease.color),
            json!(lease.epoch),
            json!(lease.state),
            json!(old_epoch),
        ],
    }
}

/// What a job Container's start answers while the lease is `promoting`: no job starts during a drain, and callers wait as for a full fleet.
pub const WAIT: &str = "the lease is promoting: no job starts until it serves";

/// Whether `lease` is promoting.
#[must_use]
pub fn promoting(lease: Option<&Lease>) -> bool {
    lease.is_some_and(|l| l.state == "promoting")
}

/// The code-check jobs D1 shows running, updated at or after `since`: what a drain waits for.
#[must_use]
pub fn running_jobs(instance: &str, repo: &str, since: u64) -> crate::control::Statement {
    crate::control::Statement {
        sql: "SELECT COUNT(*) AS n FROM jobs WHERE instance = ?1 AND repo = ?2 AND status = 'running' AND updated >= ?3",
        params: vec![json!(instance), json!(repo), json!(since)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_promoting_lease_holds_job_starts() {
        let l = |state: &str| Lease {
            color: FIRST_COLOR.into(),
            epoch: 1,
            state: state.into(),
        };
        assert!(promoting(Some(&l("promoting"))));
        assert!(!promoting(Some(&l("serving"))));
        assert!(!promoting(None));
        assert!(crate::control::no_capacity(WAIT));
    }
}
