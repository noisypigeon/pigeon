//! The control plane's git front door: the pure parts of judging a push, shared by both builds;
//! see `.claude/decisions/2026-11-06-security-git-front-door-spike.md` §2.

use crate::changes::{self, Change, State};

/// A git smart-HTTP endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// `info/refs`, the ref advertisement.
    InfoRefs,
    /// `git-upload-pack`: fetch and clone.
    UploadPack,
    /// `git-receive-pack`: push.
    ReceivePack,
}

/// `/git/<namespace>/<repo>.git/<endpoint>`'s namespace, repository and service.
#[must_use]
pub fn path(path: &str) -> Option<(&str, &str, Service)> {
    let rest = path.strip_prefix("/git/")?;
    let (ns, rest) = rest.split_once('/')?;
    let (repo, endpoint) = rest.split_once(".git/")?;
    let service = match endpoint {
        "info/refs" => Service::InfoRefs,
        "git-upload-pack" => Service::UploadPack,
        "git-receive-pack" => Service::ReceivePack,
        _ => return None,
    };
    let ok = |s: &str| !s.is_empty() && !s.contains('/');
    (ok(ns) && ok(repo)).then_some((ns, repo, service))
}

/// One ref a push updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// Its old SHA; zeros for a new ref.
    pub old: String,
    /// Its new SHA; zeros for a deletion.
    pub new: String,
    /// The ref.
    pub name: String,
}

impl RefUpdate {
    /// The change its branch names, if it is `refs/heads/change/<n>-<slug>`.
    #[must_use]
    pub fn change(&self) -> Option<(u32, String)> {
        self.name.strip_prefix("refs/heads/")?;
        changes::branch(&self.name)
    }
}

/// A push's request: its ref updates and the client's capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    /// Every ref it updates.
    pub updates: Vec<RefUpdate>,
    /// The capabilities on its first line.
    pub caps: Vec<String>,
}

impl Push {
    /// Whether the client asked for side-band output.
    #[must_use]
    pub fn side_band(&self) -> bool {
        self.caps
            .iter()
            .any(|c| c == "side-band-64k" || c == "side-band")
    }
}

/// The pkt-lines of `body` up to its first flush; `None` when malformed.
fn lines(body: &[u8]) -> Option<Vec<&[u8]>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let len = std::str::from_utf8(body.get(at..at + 4)?).ok()?;
        let len = usize::from_str_radix(len, 16).ok()?;
        if len == 0 {
            return Some(out);
        }
        if len < 4 {
            return None;
        }
        out.push(body.get(at + 4..at + len)?);
        at += len;
    }
}

/// The ref updates at the start of a `git-receive-pack` request's body.
///
/// # Errors
///
/// A message when it isn't protocol v1's command list.
pub fn commands(body: &[u8]) -> Result<Push, String> {
    let lines = lines(body).ok_or("the push's commands are not pkt-lines")?;
    let mut push = Push {
        updates: Vec::new(),
        caps: Vec::new(),
    };
    for line in lines {
        let line = std::str::from_utf8(line).map_err(|_| "a command is not UTF-8")?;
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line.starts_with("shallow ") {
            continue;
        }
        let command = match line.split_once('\0') {
            Some((c, caps)) if push.updates.is_empty() => {
                push.caps = caps.split(' ').map(str::to_string).collect();
                c
            }
            Some(_) => return Err("capabilities after the first command".into()),
            None => line,
        };
        let mut words = command.split(' ');
        let (Some(old), Some(new), Some(name), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            return Err(format!("`{command}` is not a ref update"));
        };
        if !changes::is_sha(old) || !changes::is_sha(new) {
            return Err(format!("`{command}` is not a ref update"));
        }
        push.updates.push(RefUpdate {
            old: old.into(),
            new: new.into(),
            name: name.into(),
        });
    }
    if push.updates.is_empty() {
        return Err("the push updates no ref".into());
    }
    Ok(push)
}

/// Whether `update` may be pushed, given the change its branch names, if any.
///
/// # Errors
///
/// Why not, for git's `ng` line.
pub fn judge(update: &RefUpdate, change: Option<&Change>) -> Result<(), String> {
    if update.new.bytes().all(|b| b == b'0') {
        return Err("branches are deleted by the control plane".into());
    }
    let Some((number, slug)) = update.change() else {
        return Err("only change/<n>-<slug> branches are pushed here".into());
    };
    let Some(change) = change.filter(|c| c.number == number) else {
        return Err(format!("no change {number}"));
    };
    if change.slug != slug {
        return Err(format!("change {number}'s branch is {}", change.branch()));
    }
    match change.state {
        State::Proposed => Err(format!("change {number}'s plan is not approved")),
        State::Merged | State::Closed => Err(format!("change {number} is {}", change.state.name())),
        _ => Ok(()),
    }
}

fn pkt(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(format!("{:04x}", data.len() + 4).as_bytes());
    out.extend_from_slice(data);
}

/// The report-status answer refusing every ref of `push`: `ng <ref> <reason>`, each ref's
/// reason from `refusals` or, for a ref that passed, that another was refused; on side-band
/// channel 1 when the client asked for it.
#[must_use]
pub fn refusal(push: &Push, refusals: &[(String, String)]) -> Vec<u8> {
    let mut report = Vec::new();
    pkt(&mut report, b"unpack ok\n");
    for u in &push.updates {
        let why = refusals
            .iter()
            .find(|(name, _)| *name == u.name)
            .map_or("another ref in this push was refused", |(_, why)| why);
        let why = why.replace('\n', " ");
        pkt(&mut report, format!("ng {} {why}\n", u.name).as_bytes());
    }
    report.extend_from_slice(b"0000");
    if !push.side_band() {
        return report;
    }
    let mut out = Vec::new();
    for chunk in report.chunks(65515) {
        let mut data = vec![1u8];
        data.extend_from_slice(chunk);
        pkt(&mut out, &data);
    }
    out.extend_from_slice(b"0000");
    out
}
