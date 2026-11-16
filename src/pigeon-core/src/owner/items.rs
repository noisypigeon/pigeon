//! Owner items: a decision only the owner makes, or an action only the owner can take, filed by a
//! session and answered from the owner's device; see
//! `.claude/decisions/2026-11-15-security-standing-auto-owner-flags.md` §4.

use serde::{Deserialize, Serialize};

/// The open items a session may hold at once.
pub const MAX_PER_SESSION: usize = 5;
/// The longest title, in characters.
const TITLE: usize = 120;
/// The longest detail or answer, in characters.
const DETAIL: usize = 4_000;
/// The most options a decision offers.
const OPTIONS: usize = 4;
/// The longest option, in characters.
const OPTION: usize = 80;
/// How many closed items are kept, newest last.
const KEEP: usize = 50;

/// What the owner is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A choice only the owner can make.
    Decision,
    /// Something only the owner can do.
    Action,
}

impl Kind {
    /// The kind `s` names.
    ///
    /// # Errors
    ///
    /// When it is neither `decision` nor `action`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "decision" => Ok(Self::Decision),
            "action" => Ok(Self::Action),
            other => Err(format!(
                "`kind` is `decision` or `action`, not `{other}`; a destructive act is asked where it happens"
            )),
        }
    }

    /// The push's title.
    #[must_use]
    pub fn headline(self) -> &'static str {
        match self {
            Self::Decision => "Decision needed",
            Self::Action => "Action for you",
        }
    }

    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Action => "action",
        }
    }
}

/// One item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Its number.
    pub id: u32,
    /// What it asks for.
    pub kind: Kind,
    /// The session that filed it.
    pub session: String,
    /// The change it is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<u32>,
    /// One line.
    pub title: String,
    /// What the owner needs to know.
    pub detail: String,
    /// A decision's choices; free text is always accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// When it was filed, in Unix seconds.
    pub opened: u64,
    /// The owner's answer, once given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// Who answered, or `withdrawn by <session>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// When it was answered or withdrawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<u64>,
}

impl Item {
    /// Whether it still waits on the owner.
    #[must_use]
    pub fn open(&self) -> bool {
        self.closed.is_none()
    }

    /// Its digest line at `now`.
    #[must_use]
    pub fn line(&self, now: u64) -> String {
        let age = now.saturating_sub(self.opened) / 60;
        let on = self.change.map(|n| format!(" on !{n}")).unwrap_or_default();
        format!(
            "{} #{}{on} from {}, {}h{:02}m: {}",
            self.kind.name(),
            self.id,
            crate::sessions::session_id(&self.session),
            age / 60,
            age % 60,
            self.title
        )
    }
}

/// What a session files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// What it asks for.
    pub kind: Kind,
    /// The session filing it.
    pub session: String,
    /// The change it is about.
    pub change: Option<u32>,
    /// One line.
    pub title: String,
    /// What the owner needs to know.
    pub detail: String,
    /// A decision's choices.
    pub options: Vec<String>,
}

/// Every item: the open ones, and the closed ones kept for the record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Items {
    /// The items, oldest first.
    #[serde(default)]
    pub list: Vec<Item>,
    /// The last number given.
    #[serde(default)]
    pub next: u32,
}

fn bounded(what: &str, s: &str, max: usize) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(format!("`{what}` is empty"));
    }
    if s.chars().count() > max {
        return Err(format!("`{what}` is longer than {max} characters"));
    }
    Ok(s.to_string())
}

impl Items {
    /// The open items, oldest first.
    #[must_use]
    pub fn open(&self) -> Vec<&Item> {
        self.list.iter().filter(|i| i.open()).collect()
    }

    /// Files `ask` at `now`.
    ///
    /// # Errors
    ///
    /// When a field is empty or too long, an action offers options, a decision offers more than
    /// four, or its session already holds [`MAX_PER_SESSION`] open items.
    pub fn ask(&self, ask: &Ask, now: u64) -> Result<(Self, Item), String> {
        let session = bounded("session", &ask.session, DETAIL)?;
        let title = bounded("title", &ask.title, TITLE)?;
        let detail = bounded("detail", &ask.detail, DETAIL)?;
        if ask.kind == Kind::Action && !ask.options.is_empty() {
            return Err("an action offers no `options`; the owner marks it done".into());
        }
        if ask.options.len() > OPTIONS {
            return Err(format!("a decision offers at most {OPTIONS} `options`"));
        }
        let options = ask
            .options
            .iter()
            .map(|o| bounded("option", o, OPTION))
            .collect::<Result<Vec<_>, _>>()?;
        let held = self.open().iter().filter(|i| i.session == session).count();
        if held >= MAX_PER_SESSION {
            return Err(format!(
                "wait: session {session} has {MAX_PER_SESSION} items open for the owner; one answered wakes you (`owner.answered`)"
            ));
        }
        let mut next = self.clone();
        next.next += 1;
        let item = Item {
            id: next.next,
            kind: ask.kind,
            session,
            change: ask.change,
            title,
            detail,
            options,
            opened: now,
            answer: None,
            by: None,
            closed: None,
        };
        next.list.push(item.clone());
        Ok((next, item))
    }

    /// The owner `by` answers item `id` with `answer` at `now`.
    ///
    /// # Errors
    ///
    /// When it isn't open, or the answer is empty or too long.
    pub fn answer(
        &self,
        id: u32,
        answer: &str,
        by: &str,
        now: u64,
    ) -> Result<(Self, Item), String> {
        let answer = bounded("answer", answer, DETAIL)?;
        self.close(id, now, |i| {
            (i.answer, i.by) = (Some(answer.clone()), Some(by.to_string()));
            Ok(())
        })
    }

    /// Session `session` withdraws its own item `id` at `now`.
    ///
    /// # Errors
    ///
    /// When it isn't open, or another session filed it.
    pub fn withdraw(&self, id: u32, session: &str, now: u64) -> Result<(Self, Item), String> {
        self.close(id, now, |i| {
            if i.session != session {
                return Err(format!("item #{id} is another session's"));
            }
            i.by = Some(format!("withdrawn by {session}"));
            Ok(())
        })
    }

    fn close(
        &self,
        id: u32,
        now: u64,
        how: impl FnOnce(&mut Item) -> Result<(), String>,
    ) -> Result<(Self, Item), String> {
        let mut next = self.clone();
        let Some(item) = next.list.iter_mut().find(|i| i.id == id && i.open()) else {
            return Err(format!("no open item #{id}"));
        };
        how(item)?;
        item.closed = Some(now);
        let item = item.clone();
        let closed = next.list.iter().filter(|i| !i.open()).count();
        let mut drop = closed.saturating_sub(KEEP);
        next.list.retain(|i| {
            let gone = drop > 0 && !i.open();
            if gone {
                drop -= 1;
            }
            !gone
        });
        Ok((next, item))
    }

    /// The digest's lines at `now`: one per open item, oldest first.
    #[must_use]
    pub fn lines(&self, now: u64) -> Vec<String> {
        self.open().iter().map(|i| i.line(now)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Ask, Items, Kind, MAX_PER_SESSION};

    const S: &str = "https://claude.ai/code/session_01ABC";

    fn ask(kind: Kind, options: &[&str]) -> Ask {
        Ask {
            kind,
            session: S.into(),
            change: Some(7),
            title: "Which region?".into(),
            detail: "The bucket needs one.".into(),
            options: options.iter().map(|o| (*o).to_string()).collect(),
        }
    }

    #[test]
    fn a_decision_is_filed_answered_once_and_kept() {
        let (all, item) = Items::default()
            .ask(&ask(Kind::Decision, &["eu", "us"]), 100)
            .expect("filed");
        assert_eq!((item.id, item.open()), (1, true));
        assert_eq!(all.open().len(), 1);
        let (all, done) = all.answer(1, " eu ", "owner", 400).expect("answered");
        assert_eq!(
            (done.answer.as_deref(), done.by.as_deref(), done.closed),
            (Some("eu"), Some("owner"), Some(400))
        );
        assert_eq!(all.open().len(), 0);
        assert_eq!(all.list.len(), 1, "kept for the record");
        assert!(all.answer(1, "us", "owner", 500).is_err(), "once");
    }

    #[test]
    fn bad_asks_are_refused() {
        let none = Items::default();
        assert!(none.ask(&ask(Kind::Action, &["x"]), 0).is_err());
        assert!(
            none.ask(&ask(Kind::Decision, &["a", "b", "c", "d", "e"]), 0)
                .is_err()
        );
        let mut empty = ask(Kind::Decision, &[]);
        empty.title = "  ".into();
        assert!(none.ask(&empty, 0).is_err());
        assert!(Kind::parse("destructive").is_err());
        assert_eq!(Kind::parse("action"), Ok(Kind::Action));
        assert!(none.answer(9, "x", "owner", 0).is_err());
        let (one, _) = none.ask(&ask(Kind::Action, &[]), 0).expect("filed");
        assert!(one.answer(1, " ", "owner", 0).is_err(), "an empty answer");
    }

    #[test]
    fn a_session_holds_at_most_five_and_withdraws_only_its_own() {
        let mut all = Items::default();
        for _ in 0..MAX_PER_SESSION {
            all = all.ask(&ask(Kind::Action, &[]), 0).expect("filed").0;
        }
        let refused = all.ask(&ask(Kind::Action, &[]), 0).expect_err("full");
        assert!(refused.starts_with("wait: "), "{refused}");
        assert!(all.withdraw(1, "https://claude.ai/code/other", 5).is_err());
        let (all, w) = all.withdraw(1, S, 5).expect("withdrawn");
        assert_eq!(w.by.as_deref(), Some(format!("withdrawn by {S}").as_str()));
        assert!(all.ask(&ask(Kind::Action, &[]), 6).is_ok(), "room again");
    }

    #[test]
    fn the_digest_names_each_open_item_with_its_age() {
        let (all, _) = Items::default()
            .ask(&ask(Kind::Decision, &["eu"]), 0)
            .expect("filed");
        assert_eq!(
            all.lines(2 * 3_600 + 5 * 60),
            vec!["decision #1 on !7 from session_01ABC, 2h05m: Which region?".to_string()]
        );
    }
}
