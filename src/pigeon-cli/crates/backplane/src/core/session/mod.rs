//! The `session` domain: what a session's event follower and hooks decide and print; see
//! `.claude/decisions/2026-11-13-development-rust-session-event-follower.md`.

use serde::Deserialize;
use serde_json::Value;

use crate::core::change::stage::{Needs, Stage};

#[cfg(test)]
mod tests;

/// The hold a poll asks for, in seconds: under the proxy's ~30 s cut (§1).
pub const HOLD: u64 = 20;
/// The lowest hold a cut lowers it to, in seconds.
pub const HOLD_MIN: u64 = 5;
/// The longest backoff, in seconds.
pub const BACKOFF_MAX: u64 = 60;
/// The failure in a row that is said first, then every [`LOUD_EVERY`].
pub const LOUD_AFTER: u32 = 5;
/// How often failures in a row are said again, after [`LOUD_AFTER`].
pub const LOUD_EVERY: u32 = 60;
/// How long refusals alone go on before the follower stops, in seconds.
pub const GIVE_UP: u64 = 10 * 60;
/// How old the status cache may grow while answers keep coming, in seconds.
pub const STATUS_EVERY: u64 = 60;
/// How old the status cache is before the status line says `stale`, in seconds.
pub const STALE: u64 = 10 * 60;
/// The exit code after [`GIVE_UP`].
pub const EXIT_REFUSED: i32 = 3;

/// `CLAUDE.md`'s start steps, for a session with no `~/.config/pigeon/session.json`.
pub const START: &str = "No pigeon session yet. Per CLAUDE.md:
1. `git remote add artifacts https://platform.pigeon.dev/git/pigeon/pigeon.git`; fetch its main and branch from it.
2. `session_start` with this session's URL, then write {\"label\": ..., \"ticket\": ...} to ~/.config/pigeon/session.json.
3. `change_propose` with the ticket before writing the change; follow with `session.py follow`.";

/// One event, as the follower reads it; unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Event {
    /// Its number.
    pub id: u64,
    /// What happened.
    pub kind: String,
    /// The change it is about.
    #[serde(default)]
    pub change: Option<u32>,
    /// What happened, in a line.
    #[serde(default)]
    pub summary: String,
    /// The change's stage as the event was appended.
    #[serde(default)]
    pub stage: Option<Stage>,
}

/// `GET /v0/events`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Answer {
    /// The log's last event.
    pub last: u64,
    /// The events the poll asked for.
    pub events: Vec<Event>,
}

/// What came back from a poll, as the shell saw it.
#[derive(Debug, Clone, Copy)]
pub struct Reply<'a> {
    /// The HTTP status.
    pub status: u16,
    /// Its `Content-Type`.
    pub content_type: &'a str,
    /// Whether it carries `Cf-Ray`: Cloudflare answered.
    pub cloudflare: bool,
    /// Its body.
    pub body: &'a str,
}

/// A poll's outcome (§2's table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The log answered, perhaps with no events.
    Answer(Answer),
    /// The session's proxy cut the poll.
    Cut,
    /// The request was refused: credentials, the route or a parameter.
    Refused(String),
    /// The server failed.
    Server(String),
    /// No answer: a transport error, a timeout, or a body that isn't the answer.
    Unreachable(String),
}

impl Outcome {
    fn class(&self) -> (&'static str, &str) {
        match self {
            Self::Answer(_) => ("answer", ""),
            Self::Cut => ("cut", "the proxy cut the poll"),
            Self::Refused(d) => ("refused", d),
            Self::Server(d) => ("server", d),
            Self::Unreachable(d) => ("unreachable", d),
        }
    }
}

/// The body's first line, at most 80 characters.
fn snippet(body: &str) -> String {
    body.lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(80)
        .collect()
}

/// Classifies a reply.
#[must_use]
pub fn classify(r: &Reply<'_>) -> Outcome {
    let detail = || format!("{} {}", r.status, snippet(r.body));
    match r.status {
        200 => serde_json::from_str::<Answer>(r.body).map_or_else(
            |_| Outcome::Unreachable(format!("200 that isn't an answer: {}", snippet(r.body))),
            Outcome::Answer,
        ),
        502 if !r.cloudflare
            && r.content_type.starts_with("text/plain")
            && r.body.trim() == "upstream request failed" =>
        {
            Outcome::Cut
        }
        400 | 401 | 403 | 404 => Outcome::Refused(detail()),
        _ => Outcome::Server(detail()),
    }
}

/// Where the next poll starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// From the session's cursor on the control plane.
    Session,
    /// After this event.
    At(u64),
}

/// Percent-encodes `v` for a query.
#[must_use]
pub fn quote(v: &str) -> String {
    v.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// The poll's path and query.
#[must_use]
pub fn path(label: &str, cursor: Cursor, hold: u64) -> String {
    let after = match cursor {
        Cursor::Session => "session".to_string(),
        Cursor::At(n) => n.to_string(),
    };
    format!(
        "/v0/events?session={}&after={after}&wake=agent&seconds={hold}",
        quote(label)
    )
}

/// The line an event prints, as `session.py` prints it.
#[must_use]
pub fn event_line(e: &Event) -> String {
    let n = e.change.map(|c| format!("!{c} ")).unwrap_or_default();
    let at = e
        .stage
        .as_ref()
        .map(|s| format!("{} needs={} ", s.stage, needs(s.needs)))
        .unwrap_or_default();
    format!("{} {n}{at}{}: {}", e.id, e.kind, e.summary)
}

fn needs(n: Needs) -> String {
    serde_json::to_value(n)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A failure's backoff, in seconds: `1..=min(60, 2^n)` for the n-th in a row, from `random`.
#[must_use]
pub fn backoff(failures: u32, random: u32) -> u64 {
    let cap = 2u64.saturating_pow(failures).clamp(1, BACKOFF_MAX);
    1 + u64::from(random) % cap
}

/// The follower's state; owned by its process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    /// Where the next poll starts.
    pub cursor: Cursor,
    /// The hold it asks for, in seconds.
    pub hold: u64,
    /// Failures in a row.
    pub failures: u32,
    /// When the refusals in a row began.
    pub refused_since: Option<u64>,
    /// When the status cache was last written.
    pub status_at: u64,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            cursor: Cursor::Session,
            hold: HOLD,
            failures: 0,
            refused_since: None,
            status_at: 0,
        }
    }
}

/// What the shell does after a poll, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Print a line on stdout.
    Print(String),
    /// Refresh the status cache from `session.status`.
    Status,
    /// Sleep this many seconds.
    Sleep(u64),
    /// Exit with this code.
    Exit(i32),
}

/// The next state and effects after a poll's `outcome`, at `now`, with `random` for jitter.
#[must_use]
pub fn step(s: &Follow, outcome: Outcome, now: u64, random: u32) -> (Follow, Vec<Effect>) {
    match outcome {
        Outcome::Answer(a) => answered(s, &a, now),
        Outcome::Cut if s.hold > HOLD_MIN => {
            let hold = (s.hold / 2).max(HOLD_MIN);
            let say = format!("events: the proxy cut a {}s poll; holding {hold}s", s.hold);
            let next = Follow {
                hold,
                refused_since: None,
                ..s.clone()
            };
            (next, vec![Effect::Print(say)])
        }
        o => failed(s, &o, now, random),
    }
}

fn answered(s: &Follow, a: &Answer, now: u64) -> (Follow, Vec<Effect>) {
    let again = (s.failures >= LOUD_AFTER).then(|| Effect::Print("events: following again".into()));
    let lines = a.events.iter().map(|e| Effect::Print(event_line(e)));
    let cursor = match (a.events.last(), s.cursor) {
        (Some(e), _) => Cursor::At(e.id),
        (None, Cursor::Session) => Cursor::At(a.last),
        (None, c) => c,
    };
    let refresh = !a.events.is_empty() || now.saturating_sub(s.status_at) >= STATUS_EVERY;
    let next = Follow {
        cursor,
        hold: s.hold,
        failures: 0,
        refused_since: None,
        status_at: if refresh { now } else { s.status_at },
    };
    let effects = again
        .into_iter()
        .chain(lines)
        .chain(refresh.then_some(Effect::Status))
        .collect();
    (next, effects)
}

fn failed(s: &Follow, o: &Outcome, now: u64, random: u32) -> (Follow, Vec<Effect>) {
    let failures = s.failures.saturating_add(1);
    let refused_since = matches!(o, Outcome::Refused(_)).then(|| s.refused_since.unwrap_or(now));
    let (class, detail) = o.class();
    if refused_since.is_some_and(|t| now.saturating_sub(t) >= GIVE_UP) {
        let say = format!("events: refused for 10 minutes ({detail}); stopping");
        return (
            Follow {
                failures,
                refused_since,
                ..s.clone()
            },
            vec![Effect::Print(say), Effect::Exit(EXIT_REFUSED)],
        );
    }
    let loud = failures == LOUD_AFTER
        || (failures > LOUD_AFTER && (failures - LOUD_AFTER).is_multiple_of(LOUD_EVERY));
    let say = loud.then(|| {
        Effect::Print(format!(
            "events: {failures} failed polls in a row ({class}: {detail}); still trying"
        ))
    });
    let next = Follow {
        failures,
        refused_since,
        ..s.clone()
    };
    let effects = say
        .into_iter()
        .chain([Effect::Sleep(backoff(failures, random))])
        .collect();
    (next, effects)
}

/// One of a session's changes in `session.status`.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusChange {
    /// Its number.
    pub number: u32,
    /// Its title.
    #[serde(default)]
    pub title: String,
    /// Its stage.
    pub stage: Stage,
}

/// One of a session's tasks in `session.status`.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusTask {
    /// Its id.
    pub id: Value,
    /// Its kind.
    #[serde(default)]
    pub kind: String,
    /// Its change.
    #[serde(default)]
    pub change: Value,
    /// Its note.
    #[serde(default)]
    pub note: String,
}

/// Whether the session's follower is listening.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Listening {
    /// A poll within two minutes.
    #[serde(default)]
    pub listening: bool,
}

/// `session.status`'s answer, as the cache keeps it with `at`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Status {
    /// Its open changes.
    #[serde(default)]
    pub changes: Vec<StatusChange>,
    /// Its tasks.
    #[serde(default)]
    pub tasks: Vec<StatusTask>,
    /// Its event cursor.
    #[serde(default)]
    pub events: Option<Listening>,
    /// When it was cached, in Unix seconds.
    #[serde(default)]
    pub at: u64,
}

impl Status {
    fn listening(&self) -> bool {
        self.events.as_ref().is_some_and(|e| e.listening)
    }
}

fn plain(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// The status line from the cache, at `now`.
#[must_use]
pub fn statusline(s: &Status, now: u64) -> String {
    let changes = s
        .changes
        .iter()
        .map(|c| format!("!{} {}", c.number, c.stage.stage));
    let listening = if s.listening() {
        "listening"
    } else {
        "not listening"
    };
    let stale = (now.saturating_sub(s.at) > STALE).then_some("stale".to_string());
    changes
        .chain([listening.to_string()])
        .chain(stale)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The `SessionStart` brief for `label`: from `status`, or a line saying it isn't there.
#[must_use]
pub fn brief(label: &str, status: Option<&Status>) -> String {
    let Some(s) = status else {
        return format!(
            "pigeon session {label}: the control plane didn't answer; `session_status` when it does."
        );
    };
    let waiting = s.changes.iter().filter(|c| c.stage.needs == Needs::Agent);
    let others = s.changes.iter().filter(|c| c.stage.needs != Needs::Agent);
    let changes = waiting.chain(others).map(|c| {
        let st = &c.stage;
        format!(
            "- !{} {}: {}, needs {}: {}",
            c.number,
            c.title,
            st.stage,
            needs(st.needs),
            st.next
        )
    });
    let none = s
        .changes
        .is_empty()
        .then(|| "- no open changes".to_string());
    let tasks = s.tasks.iter().map(|t| {
        format!(
            "- task {} ({}) on !{}: {}",
            plain(&t.id),
            t.kind,
            plain(&t.change),
            t.note
        )
    });
    let follow = (!s.listening()).then(|| {
        "- not following events: arm Monitor with `python3 -I tooling/session/session.py follow`"
            .to_string()
    });
    [format!("pigeon session {label}:")]
        .into_iter()
        .chain(changes)
        .chain(none)
        .chain(tasks)
        .chain(follow)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `Stop` hook's `systemMessage` JSON, when a change needs the session and it isn't
/// listening.
#[must_use]
pub fn stop(s: &Status) -> Option<String> {
    if s.listening() {
        return None;
    }
    let said = s
        .changes
        .iter()
        .filter(|c| c.stage.needs == Needs::Agent)
        .map(|c| format!("!{} ({})", c.number, c.stage.next))
        .collect::<Vec<_>>();
    (!said.is_empty()).then(|| {
        serde_json::json!({
            "systemMessage": format!(
                "pigeon: {} need this session, and nothing follows its events.",
                said.join(", ")
            )
        })
        .to_string()
    })
}
