//! The lease as this Worker reads it, and its fenced journal appends; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §A.3.

use worker::{Env, Result};

use super::platform;

/// This Worker's color: the COLOR var, else the first color.
#[must_use]
pub fn color(env: &Env) -> String {
    env.var("COLOR").map_or_else(
        |_| crate::core::lease::FIRST_COLOR.to_string(),
        |v| v.to_string(),
    )
}

/// The lease, if it can be read; a failure is logged.
pub async fn current(env: &Env) -> Option<crate::core::lease::Lease> {
    match platform::d1_rows(env, &crate::core::lease::read()).await {
        Ok(rows) => rows.first().and_then(crate::core::lease::from_row),
        Err(e) => {
            worker::console_warn!("lease: {e}");
            None
        }
    }
}

/// Whether this Worker acts.
pub async fn acting(env: &Env) -> bool {
    crate::core::lease::acting(current(env).await.as_ref(), &color(env))
}

/// Appends `entries`, written at the lease's epoch, behind its fence; with no lease, unfenced.
///
/// # Errors
///
/// When D1 fails, or the fence fails the batch.
pub async fn append(
    env: &Env,
    lease: Option<&crate::core::lease::Lease>,
    entries: Vec<crate::control::Statement>,
) -> Result<()> {
    match lease {
        Some(l) => {
            let mut batch = vec![crate::core::lease::fence(l.epoch)];
            batch.extend(entries);
            platform::d1(env, &batch).await
        }
        None => platform::d1(env, &entries).await,
    }
}
