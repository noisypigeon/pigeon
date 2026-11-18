//! The feed: the instance's event log read as typed items, each saying who acts next, filtered per reader; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §3.

use serde::{Deserialize, Serialize};

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemType {
    /// A change's approval waits.
    #[serde(rename = "approval.pending")]
    ApprovalPending,
    /// A grant waits.
    #[serde(rename = "grant.pending")]
    GrantPending,
    /// The owner is asked.
    #[serde(rename = "owner.ask")]
    OwnerAsk,
    /// The owner answered.
    #[serde(rename = "owner.answered")]
    OwnerAnswered,
    /// A change moved stage.
    #[serde(rename = "change.stage")]
    ChangeStage,
    /// A task waits.
    #[serde(rename = "task.waiting")]
    TaskWaiting,
    /// A deploy failed.
    #[serde(rename = "deploy.failed")]
    DeployFailed,
    /// A deploy passed.
    #[serde(rename = "deploy.passed")]
    DeployPassed,
    /// The pool paused.
    #[serde(rename = "pool.paused")]
    PoolPaused,
    /// The hourly digest.
    #[serde(rename = "digest")]
    Digest,
    /// A session ended.
    #[serde(rename = "session.ended")]
    SessionEnded,
    /// A provider refused a call.
    #[serde(rename = "provider.refused")]
    ProviderRefused,
    /// Anything else.
    #[serde(rename = "note")]
    Note,
}

impl ItemType {
    /// Every type, in declaration order.
    pub const ALL: [Self; 13] = [
        Self::ApprovalPending,
        Self::GrantPending,
        Self::OwnerAsk,
        Self::OwnerAnswered,
        Self::ChangeStage,
        Self::TaskWaiting,
        Self::DeployFailed,
        Self::DeployPassed,
        Self::PoolPaused,
        Self::Digest,
        Self::SessionEnded,
        Self::ProviderRefused,
        Self::Note,
    ];

    /// Its serde name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::ApprovalPending => "approval.pending",
            Self::GrantPending => "grant.pending",
            Self::OwnerAsk => "owner.ask",
            Self::OwnerAnswered => "owner.answered",
            Self::ChangeStage => "change.stage",
            Self::TaskWaiting => "task.waiting",
            Self::DeployFailed => "deploy.failed",
            Self::DeployPassed => "deploy.passed",
            Self::PoolPaused => "pool.paused",
            Self::Digest => "digest",
            Self::SessionEnded => "session.ended",
            Self::ProviderRefused => "provider.refused",
            Self::Note => "note",
        }
    }

    /// The type named `s`, the inverse of [`ItemType::name`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == s)
    }
}

/// Who acts next on an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Needs {
    /// The owner.
    Owner,
    /// The session.
    Agent,
    /// Nobody.
    None,
}

impl Needs {
    /// Its lowercase name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Agent => "agent",
            Self::None => "none",
        }
    }

    /// The needs named `s`, the inverse of [`Needs::name`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Owner, Self::Agent, Self::None]
            .into_iter()
            .find(|n| n.name() == s)
    }
}

/// The type of an event of `kind`.
#[must_use]
pub fn item_type(kind: &str) -> ItemType {
    match kind {
        "approval.pending" => ItemType::ApprovalPending,
        "grant.requested" => ItemType::GrantPending,
        "owner.asked" => ItemType::OwnerAsk,
        "owner.answered" => ItemType::OwnerAnswered,
        "task.made" => ItemType::TaskWaiting,
        "deploy.failed" => ItemType::DeployFailed,
        "deploy.passed" => ItemType::DeployPassed,
        "pool.paused" => ItemType::PoolPaused,
        "digest" => ItemType::Digest,
        "session.ended" | "session.archivable" => ItemType::SessionEnded,
        "provider.refused" => ItemType::ProviderRefused,
        k if k.starts_with("change.") || k.starts_with("merge.") => ItemType::ChangeStage,
        _ => ItemType::Note,
    }
}

/// Who acts next on event `e`, of type `t`.
#[must_use]
pub fn needs(e: &crate::events::Event, t: ItemType) -> Needs {
    let wakes = || {
        if crate::events::wakes(e) {
            Needs::Agent
        } else {
            Needs::None
        }
    };
    match t {
        ItemType::ApprovalPending
        | ItemType::GrantPending
        | ItemType::OwnerAsk
        | ItemType::DeployFailed
        | ItemType::PoolPaused
        | ItemType::Digest => Needs::Owner,
        ItemType::OwnerAnswered | ItemType::TaskWaiting | ItemType::SessionEnded => Needs::Agent,
        ItemType::ChangeStage => {
            if e.stage
                .as_ref()
                .is_some_and(|s| s.needs == crate::change::stage::Needs::Owner)
            {
                Needs::Owner
            } else {
                wakes()
            }
        }
        ItemType::DeployPassed | ItemType::ProviderRefused | ItemType::Note => wakes(),
    }
}

/// One feed item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// The event's id.
    pub id: u64,
    /// When, in Unix seconds.
    pub at: u64,
    /// What it is.
    #[serde(rename = "type")]
    pub item_type: ItemType,
    /// The event's kind, kept for producers until step 8.
    pub kind: String,
    /// Who acts next.
    pub needs: Needs,
    /// The change it is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<u32>,
    /// That change's session, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// What happened, in a line.
    pub summary: String,
    /// Shown only to the owner, from the event.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner: bool,
    /// The item it closes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolves: Option<u64>,
}

/// The item for event `e`.
#[must_use]
pub fn item(e: &crate::events::Event) -> Item {
    let t = item_type(&e.kind);
    Item {
        id: e.id,
        at: e.at,
        item_type: t,
        kind: e.kind.clone(),
        needs: needs(e, t),
        change: e.change,
        session: e.session.clone(),
        summary: e.summary.clone(),
        owner: e.owner,
        resolves: e.resolves,
    }
}

/// Who reads the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reader {
    /// The owner, who sees every item.
    Owner,
    /// A session, by label.
    Session(String),
    /// A member who isn't the owner; reads only what every reader sees until the producers name accounts, step 8.
    Member,
}

/// Whether `r` may see `i`.
#[must_use]
pub fn visible(i: &Item, r: &Reader) -> bool {
    match r {
        Reader::Owner => true,
        Reader::Session(label) => {
            !i.owner
                && i.session.as_ref().is_none_or(|s| s == label)
                && !matches!(
                    i.item_type,
                    ItemType::GrantPending
                        | ItemType::PoolPaused
                        | ItemType::Digest
                        | ItemType::ProviderRefused
                )
        }
        Reader::Member => {
            !i.owner && matches!(i.item_type, ItemType::DeployFailed | ItemType::DeployPassed)
        }
    }
}

/// Who reads: the owner, else the named session, else a member.
#[must_use]
pub fn reader(owner: bool, session: Option<&str>) -> Reader {
    if owner {
        return Reader::Owner;
    }
    match session.map(str::trim) {
        Some(s) if !s.is_empty() => Reader::Session(session.unwrap_or_default().to_string()),
        _ => Reader::Member,
    }
}

/// The needs named by a non-blank `s`, else none.
#[must_use]
pub fn parse_needs(s: Option<&str>) -> Option<Needs> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(Needs::parse)
}

/// The items still needing someone, in order: those with needs and no later item closing them.
#[must_use]
pub fn open(items: &[Item]) -> Vec<&Item> {
    items
        .iter()
        .enumerate()
        .filter(|(n, i)| {
            i.needs != Needs::None
                && !items[n + 1..].iter().any(|l| {
                    l.resolves == Some(i.id)
                        || (l.change.is_some()
                            && l.change == i.change
                            && matches!(
                                i.item_type,
                                ItemType::ChangeStage | ItemType::ApprovalPending
                            ))
                })
        })
        .map(|(_, i)| i)
        .collect()
}

/// What a read asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Only items after this id.
    pub after: u64,
    /// Only items with these needs.
    pub needs: Option<Needs>,
    /// Only the open items.
    pub open: bool,
    /// The most items; 0 means [`crate::events::PAGE`].
    pub limit: usize,
}

/// The log's items `r` may see, filtered by `q`, in id order.
#[must_use]
pub fn read(events: &[crate::events::Event], reader: &Reader, q: &Query) -> Vec<Item> {
    let seen: Vec<Item> = events
        .iter()
        .map(item)
        .filter(|i| visible(i, reader))
        .collect();
    let base: Vec<&Item> = if q.open {
        open(&seen)
    } else {
        seen.iter().collect()
    };
    let limit = if q.limit == 0 {
        crate::events::PAGE
    } else {
        q.limit
    };
    base.into_iter()
        .filter(|i| i.id > q.after && q.needs.is_none_or(|n| i.needs == n))
        .take(limit)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;
