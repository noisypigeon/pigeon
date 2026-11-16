//! A change's own listing of the owner's approval, after each save; see
//! `.claude/decisions/2026-11-15-security-owner-approvals-always-listed.md` §2.

use serde_json::json;
use worker::{Env, Result};

use super::platform::{self, Settings, now};
use crate::approve::Waiting;
use crate::core::approval::grants::{self, Grants};
use crate::core::approval::listed::{self, Action, Listed};
use crate::core::change::{Change, State};

/// The operations a change lists for the owner.
const LISTED: [&str; 2] = ["change.authorize", "change.approve"];

/// Lists, re-issues or removes `change`'s approval in the inbox, minting into `grants_key`.
pub(super) async fn reconcile(
    env: &Env,
    storage: &worker::Storage,
    grants_key: &str,
    before: Option<&Change>,
    change: &Change,
) -> Result<()> {
    let live = |c: &Change| matches!(c.state, State::Proposed | State::Checked);
    if !(live(change) || before.is_some_and(live)) {
        return Ok(());
    }
    let s = &Settings::from(env)?;
    let q = super::stages::queue(env, s).await;
    let stage = super::stages::stage(env, s, change, &q).await;
    let owed = listed::owed(change, &stage);
    let prefix = format!("/approve/{}?grant=", change.number);
    let all = super::elicit::inbox(env, s, "/approvals/list", &json!({})).await?;
    let waiting: Vec<Waiting> = serde_json::from_value(all).unwrap_or_default();
    let entry = waiting
        .into_iter()
        .find(|w| w.path.starts_with(&prefix) && LISTED.contains(&w.operation.as_str()));
    let grants: Grants = storage.get(grants_key).await?.unwrap_or_default();
    let found = entry.as_ref().map(|w| Listed {
        operation: w.operation.clone(),
        head: grants
            .0
            .get(&grants::key(
                w.path.strip_prefix(&prefix).unwrap_or_default(),
            ))
            .and_then(|g| g.head.clone()),
        expires: w.expires,
    });
    let remove = |w: &Waiting| {
        let body = json!({ "path": w.path });
        async move { super::elicit::inbox(env, s, "/approvals/remove", &body).await }
    };
    match listed::reconcile(owed.as_ref(), found.as_ref(), now()) {
        Action::Keep => Ok(()),
        Action::Remove => {
            let grant = |w: &Waiting| {
                grants.0.get(&grants::key(
                    w.path.strip_prefix(&prefix).unwrap_or_default(),
                ))
            };
            if let Some(w) = entry
                .as_ref()
                .filter(|w| !listed::applies(change, grant(w)))
            {
                remove(w).await?;
            }
            Ok(())
        }
        Action::Issue { push } => {
            let Some(o) = owed else {
                return Ok(());
            };
            let token = platform::random_hex(32)?;
            let (next, minted) =
                grants.mint_for(change, o.operation, &token, now(), grants::PAGE_TTL);
            let grant = match minted {
                Ok(g) => g,
                Err(why) => {
                    worker::console_warn!("listed !{}: {why}", change.number);
                    return Ok(());
                }
            };
            storage.put(grants_key, &next).await?;
            let path = format!("{prefix}{token}");
            let w = Waiting {
                number: change.number,
                operation: o.operation.to_string(),
                title: change.title.clone(),
                path: path.clone(),
                expires: grant.expires,
            };
            super::elicit::inbox(env, s, "/approvals/put", &json!(w)).await?;
            if let Some(old) = entry.filter(|e| e.operation != o.operation) {
                remove(&old).await?;
            }
            let said = format!(
                "`{}` on !{} waits for the owner",
                o.operation, change.number
            );
            let _ = super::events::append(
                env,
                s,
                "approval.pending",
                Some(change.number),
                None,
                &said,
                false,
            )
            .await;
            if push {
                super::push::notify(env, s, "Approval waiting", &said, &path).await;
            }
            Ok(())
        }
    }
}
