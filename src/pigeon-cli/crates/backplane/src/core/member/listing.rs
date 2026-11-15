//! The platform's private listing of personal instances, kept only in its `Instance` under
//! `instances.private`: never in a repository file, change, commit, branch or log; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md` §2 and its
//! 2026-11-15 amendments.

use serde::{Deserialize, Serialize};

/// Where the `Instance` keeps the listing.
pub const STORE: &str = "instances.private";

/// The most entries it holds.
pub const MAX: usize = 8;

/// The instance every app-shape stand-up names on its stack; the listing's `name` is the
/// owner's label, kept here alone.
pub const STACK_INSTANCE: &str = "learner";

/// The passkey purpose that proves a stand-up with the token whose SHA-256 is `token_sha` (hex).
#[must_use]
pub fn standup_purpose(token_sha: &str) -> String {
    format!("instance:standup:{token_sha}")
}

/// Where an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Its job runs.
    StandingUp,
    /// It answers.
    Ready,
    /// Its job failed; a retry resumes it.
    Failed,
}

/// One personal instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Its ID in the listing.
    pub id: String,
    /// The owner's label for it.
    pub name: String,
    /// Always `standalone`.
    pub mode: String,
    /// Always `app`.
    pub shape: String,
    /// Its `workers.dev` base URL, once ready.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Its accent colour, `#rrggbb`.
    pub accent: String,
    /// Its app features.
    pub features: Vec<String>,
    /// When it was added.
    pub created: u64,
    /// The accounts that see it: its administrators.
    pub admins: Vec<String>,
    /// Where it is.
    pub status: Status,
    /// The stand-up job, while it runs.
    #[serde(default)]
    pub job: Option<String>,
    /// Why its job failed.
    #[serde(default)]
    pub why: Option<String>,
    /// Its label under the platform's zone; see
    /// `.claude/decisions/2026-11-15-runtime-standalone-hostnames-platform-zone.md` §5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `<label>.<zone>`, its identifier and the host of its `base_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Its stack's `workers.dev` URL, which the proxy forwards to; never shown to the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// Every personal instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// The entries, oldest first.
    #[serde(default)]
    pub entries: Vec<Entry>,
}

fn clean(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(40)
        .collect()
}

fn accent_ok(a: &str) -> bool {
    a.len() == 7 && a.starts_with('#') && a[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl Listing {
    /// A stand-up of a `learner` instance labelled `name` begins as job `job`, seen by `admin`;
    /// a failed entry of the same label resumes.
    ///
    /// # Errors
    /// A bad label or accent, one already standing up, or the listing is full.
    pub fn begin(
        mut self,
        id: &str,
        name: &str,
        accent: &str,
        admin: &str,
        job: &str,
        now: u64,
    ) -> Result<(Self, Entry), String> {
        let name = clean(name);
        if name.is_empty() {
            return Err("the instance needs a name".into());
        }
        if !accent_ok(accent) {
            return Err("the accent is not #rrggbb".into());
        }
        if self.entries.iter().any(|e| e.status == Status::StandingUp) {
            return Err("an instance is already standing up".into());
        }
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.name == name && e.status == Status::Failed)
        {
            e.status = Status::StandingUp;
            e.job = Some(job.to_owned());
            e.why = None;
            let out = e.clone();
            return Ok((self, out));
        }
        if self.entries.iter().any(|e| e.name == name) {
            return Err("an instance has this name".into());
        }
        if self.entries.len() >= MAX {
            return Err(format!("at most {MAX} personal instances"));
        }
        let e = Entry {
            id: id.to_owned(),
            name,
            mode: "standalone".into(),
            shape: "app".into(),
            base_url: None,
            accent: accent.to_owned(),
            features: vec!["learner".into()],
            created: now,
            admins: vec![admin.to_owned()],
            status: Status::StandingUp,
            job: Some(job.to_owned()),
            why: None,
            label: None,
            host: None,
            origin: None,
        };
        self.entries.push(e.clone());
        Ok((self, e))
    }

    /// Job `job` finished: its entry ready at the base URL, or failed with why.
    #[must_use]
    pub fn finished(mut self, job: &str, outcome: Result<String, String>) -> Self {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.job.as_deref() == Some(job))
        {
            e.job = None;
            match outcome {
                Ok(base) if base.starts_with("https://") && base.ends_with(".workers.dev") => {
                    match &e.host {
                        Some(h) => {
                            e.base_url = Some(format!("https://{h}"));
                            e.origin = Some(base);
                        }
                        None => e.base_url = Some(base),
                    }
                    e.status = Status::Ready;
                }
                Ok(_) => {
                    e.status = Status::Failed;
                    e.why = Some("the job reported no workers.dev URL".into());
                }
                Err(why) => {
                    e.status = Status::Failed;
                    e.why = Some(why);
                }
            }
        }
        self
    }

    /// Entry `id` takes `label` under `zone`, checked by
    /// [`crate::core::platform::hosts::check_label`] against every other entry's label. A resumed
    /// entry keeps its label.
    ///
    /// # Errors
    /// Why the label can't be used, or no entry `id`.
    pub fn named(mut self, id: &str, label: &str, zone: &str) -> Result<Self, String> {
        let taken: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.id != id)
            .filter_map(|e| e.label.clone())
            .collect();
        let e = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("no such instance")?;
        if let Some(had) = &e.label {
            return if had == label {
                Ok(self)
            } else {
                Err("a resumed instance keeps its label".into())
            };
        }
        if let Some(why) =
            crate::core::platform::hosts::check_label(label, taken.iter().map(String::as_str))
        {
            return Err(why);
        }
        e.label = Some(label.to_owned());
        e.host = Some(crate::core::platform::hosts::host_of(label, zone));
        Ok(self)
    }

    /// The entries `account` sees, as the app is shown them: never an `origin`.
    #[must_use]
    pub fn shown(&self, account: &str) -> Vec<serde_json::Value> {
        self.visible(account)
            .into_iter()
            .map(|e| {
                let mut v = serde_json::to_value(e).unwrap_or_default();
                if let Some(m) = v.as_object_mut() {
                    m.remove("origin");
                }
                v
            })
            .collect()
    }

    /// The jobs still running.
    #[must_use]
    pub fn running(&self) -> Vec<String> {
        self.entries.iter().filter_map(|e| e.job.clone()).collect()
    }

    /// The entries `account` sees.
    #[must_use]
    pub fn visible(&self, account: &str) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| e.admins.iter().any(|a| a == account))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "acct_owner";

    fn begun() -> Listing {
        Listing::default()
            .begin("i1", " Practice ", "#3366ff", OWNER, "job-1", 5)
            .unwrap()
            .0
    }

    #[test]
    fn a_standup_is_listed_for_its_administrator_alone() {
        let l = begun();
        assert_eq!(l.entries[0].name, "Practice");
        assert_eq!(l.entries[0].status, Status::StandingUp);
        assert_eq!(l.running(), ["job-1"]);
        assert_eq!(l.visible(OWNER).len(), 1);
        assert_eq!(l.visible("acct_someone").len(), 0);
    }

    #[test]
    fn a_finished_job_makes_it_ready_or_failed() {
        let ready = begun().finished("job-1", Ok("https://pigeon-learner.x.workers.dev".into()));
        assert_eq!(ready.entries[0].status, Status::Ready);
        assert_eq!(ready.running(), Vec::<String>::new());
        let odd = begun().finished("job-1", Ok("https://example.com".into()));
        assert_eq!(odd.entries[0].status, Status::Failed);
        let failed = begun().finished("job-1", Err("no subdomain".into()));
        assert_eq!(failed.entries[0].why.as_deref(), Some("no subdomain"));
        let (resumed, e) = failed
            .begin("i2", "Practice", "#3366ff", OWNER, "job-2", 6)
            .unwrap();
        assert_eq!((e.id.as_str(), e.status), ("i1", Status::StandingUp));
        assert_eq!(resumed.entries.len(), 1);
    }

    #[test]
    fn one_stands_up_at_a_time_with_a_sound_label() {
        assert!(
            begun()
                .begin("i2", "Other", "#3366ff", OWNER, "j", 6)
                .is_err()
        );
        assert!(
            Listing::default()
                .begin("i", "  ", "#3366ff", OWNER, "j", 1)
                .is_err()
        );
        assert!(
            Listing::default()
                .begin("i", "x", "blue", OWNER, "j", 1)
                .is_err()
        );
        assert_eq!(standup_purpose("ab"), "instance:standup:ab");
    }

    #[test]
    fn a_labelled_instance_is_its_host_and_hides_its_origin() {
        let l = begun().named("i1", "example", "pigeon.dev").unwrap();
        assert_eq!(l.entries[0].host.as_deref(), Some("example.pigeon.dev"));
        let ready = l.finished("job-1", Ok("https://pigeon-learner.x.workers.dev".into()));
        let e = &ready.entries[0];
        assert_eq!(e.base_url.as_deref(), Some("https://example.pigeon.dev"));
        assert_eq!(
            e.origin.as_deref(),
            Some("https://pigeon-learner.x.workers.dev")
        );
        let shown = ready.shown(OWNER);
        assert_eq!(shown[0]["host"], "example.pigeon.dev");
        assert!(shown[0].get("origin").is_none(), "{shown:?}");
        let listing = serde_json::to_value(&ready).unwrap();
        // What the proxy reads, core::platform::hosts::origin_of.
        assert_eq!(listing["entries"][0]["host"], "example.pigeon.dev");
        assert_eq!(
            listing["entries"][0]["origin"],
            "https://pigeon-learner.x.workers.dev"
        );
    }

    #[test]
    fn a_label_is_checked_unique_and_kept_on_resume() {
        assert!(begun().named("i1", "platform", "pigeon.dev").is_err());
        assert!(begun().named("nope", "example", "pigeon.dev").is_err());
        let failed = begun()
            .named("i1", "example", "pigeon.dev")
            .unwrap()
            .finished("job-1", Err("x".into()));
        let (resumed, _) = failed
            .begin("i2", "Practice", "#3366ff", OWNER, "job-2", 6)
            .unwrap();
        assert!(resumed.clone().named("i1", "example", "pigeon.dev").is_ok());
        assert_eq!(
            resumed
                .clone()
                .named("i1", "other", "pigeon.dev")
                .unwrap_err(),
            "a resumed instance keeps its label"
        );
        let (two, _) = resumed
            .finished("job-2", Err("y".into()))
            .begin("i3", "Second", "#3366ff", OWNER, "job-3", 7)
            .unwrap();
        assert_eq!(
            two.named("i3", "example", "pigeon.dev").unwrap_err(),
            "that label is taken"
        );
    }
}
