//! The mirror to GitHub: `MainRun`'s state for it, the refs a remote advertises, and the
//! GitHub App's token request; see
//! `.claude/decisions/2026-11-08-development-github-mirror-on-merge.md`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::changes;

/// How often both sides' refs are compared, in seconds.
pub const RECONCILE: u64 = 3600;

/// How long GitHub may stay behind before the digest names it, in seconds.
pub const BEHIND: u64 = 24 * 3600;

/// The waits after the first, second and third failure; then [`RECONCILE`].
const BACKOFF: [u64; 3] = [60, 300, 1800];

/// The newest mirror job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Last {
    /// The commit of `main` it mirrored.
    pub sha: String,
    /// Whether it passed.
    pub ok: bool,
    /// When it ended, in Unix seconds.
    pub at: u64,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// The mirror's state, as `MainRun` keeps it: at most one job, always of the newest push.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirroring {
    /// The newest commit of `main` GitHub should hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The last commit mirrored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirrored: Option<String>,
    /// The job in progress, and its attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<(String, u32)>,
    /// Failures since the last success.
    #[serde(default)]
    pub failures: u32,
    /// No job starts before this, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
    /// When both sides' refs were last compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    /// Since when GitHub has lacked what Artifacts holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind_since: Option<u64>,
    /// What GitHub holds that Artifacts lacks, which no job writes over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<String>,
    /// The newest job's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Last>,
}

impl Mirroring {
    /// `main` moved to `sha`, at `at`.
    pub fn pushed(&mut self, sha: &str, at: u64) {
        if changes::is_sha(sha) {
            self.target = Some(sha.to_string());
            if self.mirrored.as_deref() != Some(sha) {
                self.behind_since.get_or_insert(at);
            }
        }
    }

    /// The commit to mirror now: none while a job runs, before a retry is due, or when the
    /// newest push is mirrored.
    pub fn start(&mut self, now: u64) -> Option<String> {
        if self.running.is_some() || self.retry_at.is_some_and(|t| now < t) {
            return None;
        }
        let target = self.target.clone()?;
        if self.mirrored.as_deref() == Some(target.as_str()) {
            return None;
        }
        self.running = Some((target.clone(), 1));
        Some(target)
    }

    /// The running job ended: `ok`, or failed for `why`, at `at`.
    pub fn finished(&mut self, ok: bool, why: &str, at: u64) {
        let Some((sha, _)) = self.running.take() else {
            return;
        };
        if ok {
            self.failures = 0;
            self.retry_at = None;
            if self.target.as_deref() == Some(sha.as_str()) {
                self.behind_since = None;
            }
            self.drift = None;
            self.mirrored = Some(sha.clone());
        } else {
            self.failures += 1;
            let wait = usize::try_from(self.failures - 1)
                .ok()
                .and_then(|i| BACKOFF.get(i).copied())
                .unwrap_or(RECONCILE);
            self.retry_at = Some(at + wait);
            if why.contains("lacks") {
                self.drift = Some(why.to_string());
            }
        }
        self.last = Some(Last {
            sha,
            ok,
            at,
            why: if ok { String::new() } else { why.to_string() },
        });
    }

    /// The running job's Container was lost: the same job, again, until its attempts are used.
    pub fn lost(&mut self, at: u64) -> Option<String> {
        let (sha, attempts) = self.running.clone()?;
        if attempts < crate::runs::MAX_ATTEMPTS {
            self.running = Some((sha.clone(), attempts + 1));
            return Some(sha);
        }
        self.finished(false, "the mirror's Container was lost", at);
        None
    }

    /// Mirror the newest push again now, as `deploy_retry` asks.
    pub fn retry(&mut self, now: u64) -> Option<String> {
        self.retry_at = None;
        self.start(now)
    }

    /// Whether both sides' refs are due to be compared.
    #[must_use]
    pub fn reconcile_due(&self, now: u64) -> bool {
        self.running.is_none()
            && self
                .checked_at
                .is_none_or(|c| now >= c + RECONCILE.saturating_sub(60))
    }

    /// Both sides' refs were compared at `at`: Artifacts' `ours` and GitHub's `theirs`, each
    /// `(ref, sha)`, with `branch` the default branch. Whatever Artifacts holds that GitHub
    /// lacks becomes the target; what GitHub alone holds is drift, never written over.
    pub fn reconciled(
        &mut self,
        ours: &[(String, String)],
        theirs: &[(String, String)],
        branch: &str,
        at: u64,
    ) {
        self.checked_at = Some(at);
        let head = format!("refs/heads/{branch}");
        let find = |refs: &[(String, String)], name: &str| {
            refs.iter().find(|(r, _)| r == name).map(|(_, s)| s.clone())
        };
        let Some(our_head) = find(ours, &head) else {
            return;
        };
        let tags = |refs: &[(String, String)]| -> Vec<String> {
            refs.iter()
                .filter(|(r, _)| r.starts_with("refs/tags/"))
                .map(|(r, _)| r.clone())
                .collect()
        };
        let (our_tags, their_tags) = (tags(ours), tags(theirs));
        let missing = our_tags.iter().any(|t| !their_tags.contains(t));
        let extra = their_tags.iter().filter(|t| !our_tags.contains(t)).count();
        let same = find(theirs, &head).as_deref() == Some(our_head.as_str());
        self.target = Some(our_head.clone());
        if same && !missing {
            self.mirrored = Some(our_head);
            self.behind_since = None;
        } else {
            if self.mirrored.as_deref() == Some(our_head.as_str()) {
                self.mirrored = None;
            }
            self.behind_since.get_or_insert(at);
        }
        self.drift = (extra > 0).then(|| format!("GitHub holds {extra} tag(s) Artifacts lacks"));
    }

    /// When the alarm should next fire for the mirror, if a retry waits.
    #[must_use]
    pub fn wake_at(&self) -> Option<u64> {
        if self.running.is_some() || self.target == self.mirrored {
            return None;
        }
        self.retry_at
    }

    /// Why the digest names the mirror, if it should: GitHub holds what Artifacts lacks, or has
    /// been behind longer than [`BEHIND`].
    #[must_use]
    pub fn behind(&self, now: u64) -> Option<String> {
        if let Some(d) = &self.drift {
            return Some(format!("mirror: {d}"));
        }
        let since = self.behind_since?;
        (now >= since + BEHIND).then(|| {
            let why = self
                .last
                .as_ref()
                .filter(|l| !l.ok)
                .map_or_else(String::new, |l| format!(": {}", l.why));
            format!(
                "mirror: GitHub has been behind for {} hours{why}",
                (now - since) / 3600
            )
        })
    }
}

/// The refs a smart-HTTP `info/refs?service=git-upload-pack` answer advertises, `(ref, sha)`,
/// without peeled tags; `None` when it isn't one.
#[must_use]
pub fn advertised(body: &[u8]) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = body;
    let mut first = true;
    while rest.len() >= 4 {
        let len = usize::from_str_radix(std::str::from_utf8(&rest[..4]).ok()?, 16).ok()?;
        if len == 0 {
            rest = &rest[4..];
            continue;
        }
        if len < 4 || len > rest.len() {
            return None;
        }
        let line = std::str::from_utf8(&rest[4..len]).ok()?;
        rest = &rest[len..];
        if first {
            first = false;
            if line.starts_with("# service=") {
                continue;
            }
        }
        let line = line.split('\0').next()?.trim_end();
        let (sha, name) = line.split_once(' ')?;
        if sha.len() != 40 || name.ends_with("^{}") || name == "capabilities^{}" {
            continue;
        }
        out.push((name.to_string(), sha.to_string()));
    }
    Some(out)
}

/// base64url without padding.
#[must_use]
pub fn base64url(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// The GitHub App JWT's signing input for App `app_id` at `now`: valid from a minute ago
/// for nine minutes, under GitHub's ten.
#[must_use]
pub fn jwt_input(app_id: &str, now: u64) -> String {
    let header = json!({ "alg": "RS256", "typ": "JWT" });
    let claims = json!({ "iat": now.saturating_sub(60), "exp": now + 540, "iss": app_id });
    format!(
        "{}.{}",
        base64url(header.to_string().as_bytes()),
        base64url(claims.to_string().as_bytes())
    )
}

/// The DER of a PKCS#8 PEM private key, as `WebCrypto` imports it.
///
/// # Errors
///
/// When it isn't PKCS#8, as a PKCS#1 `RSA PRIVATE KEY` isn't, or isn't base64.
pub fn pkcs8_der(pem: &str) -> Result<Vec<u8>, String> {
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        return Err(
            "the App key is PKCS#1; convert it with `openssl pkcs8 -topk8 -nocrypt -in key.pem`"
                .into(),
        );
    }
    if !pem.contains("BEGIN PRIVATE KEY") {
        return Err("the App key is not a PEM `PRIVATE KEY`".into());
    }
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace() && *c != '=')
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect();
    crate::access::base64url(&body).map_err(|_| "the App key is not base64".to_string())
}

/// `(owner, repo)` of a `https://github.com/<owner>/<repo>` remote.
#[must_use]
pub fn github_repo(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("https://github.com/")?;
    let (owner, repo) = path.trim_end_matches('/').split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| (owner.to_string(), repo.to_string()))
}

/// The installation-token request: `repo` only, with what a mirror push needs.
#[must_use]
pub fn token_request(repo: &str) -> Value {
    json!({
        "repositories": [repo],
        "permissions": { "contents": "write", "workflows": "write" },
    })
}

/// The installation token request for dispatching and following `ci.yml` on `repo`: Actions
/// only; see `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §4.
#[must_use]
pub fn actions_token_request(repo: &str) -> Value {
    json!({
        "repositories": [repo],
        "permissions": { "actions": "write" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn refs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(r, s)| ((*r).to_string(), (*s).to_string()))
            .collect()
    }

    #[test]
    fn a_burst_of_pushes_mirrors_the_newest_once() {
        let mut m = Mirroring::default();
        m.pushed(A, 10);
        assert_eq!(m.start(10).as_deref(), Some(A));
        m.pushed(B, 11);
        assert_eq!(m.start(11), None, "one job at a time");
        m.finished(true, "", 12);
        assert_eq!(m.mirrored.as_deref(), Some(A));
        assert!(m.behind_since.is_some(), "B is still behind");
        assert_eq!(m.start(12).as_deref(), Some(B));
        m.finished(true, "", 13);
        assert_eq!(m.start(13), None);
        assert_eq!(m.behind_since, None);
    }

    #[test]
    fn a_failure_backs_off_then_retries() {
        let mut m = Mirroring::default();
        m.pushed(A, 0);
        m.start(0);
        m.finished(false, "github is down", 100);
        assert_eq!(m.start(130), None);
        assert_eq!(m.wake_at(), Some(160));
        assert_eq!(m.start(160).as_deref(), Some(A));
        m.finished(false, "github is down", 200);
        assert_eq!(m.retry_at, Some(500));
        m.start(500);
        m.finished(false, "x", 500);
        m.start(2300);
        m.finished(false, "x", 2300);
        assert_eq!(m.retry_at, Some(2300 + RECONCILE));
        assert_eq!(
            m.retry(2301).as_deref(),
            Some(A),
            "deploy_retry skips the wait"
        );
    }

    #[test]
    fn a_lost_container_retries_then_fails() {
        let mut m = Mirroring::default();
        m.pushed(A, 0);
        m.start(0);
        assert_eq!(m.lost(1).as_deref(), Some(A));
        assert_eq!(m.lost(2).as_deref(), Some(A));
        assert_eq!(m.lost(3), None);
        assert_eq!(m.failures, 1);
        assert!(m.last.as_ref().is_some_and(|l| !l.ok));
    }

    #[test]
    fn a_reconcile_of_equal_sides_starts_nothing() {
        let mut m = Mirroring::default();
        let both = refs(&[("refs/heads/main", A), ("refs/tags/v1", B)]);
        m.reconciled(&both, &both, "main", 50);
        assert_eq!(m.start(50), None);
        assert_eq!(m.behind(50 + BEHIND * 2), None);
        assert!(!m.reconcile_due(50 + 60));
        assert!(m.reconcile_due(50 + RECONCILE));
    }

    #[test]
    fn a_reconcile_mirrors_what_github_lacks_and_reports_what_it_alone_holds() {
        let mut m = Mirroring::default();
        let ours = refs(&[("refs/heads/main", A), ("refs/tags/v2", A)]);
        let theirs = refs(&[("refs/heads/main", B), ("refs/tags/old", B)]);
        m.reconciled(&ours, &theirs, "main", 0);
        assert_eq!(m.start(0).as_deref(), Some(A));
        assert_eq!(
            m.behind(0).as_deref(),
            Some("mirror: GitHub holds 1 tag(s) Artifacts lacks")
        );

        let mut tags_only = Mirroring {
            mirrored: Some(A.into()),
            ..Mirroring::default()
        };
        let ours = refs(&[("refs/heads/main", A), ("refs/tags/v2", A)]);
        let theirs = refs(&[("refs/heads/main", A)]);
        tags_only.reconciled(&ours, &theirs, "main", 0);
        assert_eq!(
            tags_only.start(0).as_deref(),
            Some(A),
            "a missing tag mirrors"
        );
    }

    #[test]
    fn a_mirror_behind_for_a_day_is_named() {
        let mut m = Mirroring::default();
        m.pushed(A, 0);
        m.start(0);
        m.finished(false, "403 from GitHub", 10);
        assert_eq!(m.behind(BEHIND - 1), None);
        assert_eq!(
            m.behind(BEHIND).as_deref(),
            Some("mirror: GitHub has been behind for 24 hours: 403 from GitHub")
        );
    }

    #[test]
    fn a_divergence_is_drift() {
        let mut m = Mirroring::default();
        m.pushed(A, 0);
        m.start(0);
        m.finished(
            false,
            "https://github.com/x/y has commits or tags Artifacts lacks; run `pigeon cobot git sync y` first",
            1,
        );
        assert!(m.behind(1).is_some_and(|b| b.contains("Artifacts lacks")));
    }

    #[test]
    fn advertised_refs_are_read_from_info_refs() {
        let mut body = Vec::new();
        let mut pkt = |s: &str| body.extend(format!("{:04x}{s}", s.len() + 4).into_bytes());
        pkt("# service=git-upload-pack\n");
        body.extend(b"0000");
        let mut pkt = |s: &str| body.extend(format!("{:04x}{s}", s.len() + 4).into_bytes());
        pkt(&format!("{A} HEAD\0multi_ack side-band-64k\n"));
        pkt(&format!("{A} refs/heads/main\n"));
        pkt(&format!("{B} refs/tags/v1\n"));
        pkt(&format!("{A} refs/tags/v1^{{}}\n"));
        body.extend(b"0000");
        assert_eq!(
            advertised(&body),
            Some(refs(&[
                ("HEAD", A),
                ("refs/heads/main", A),
                ("refs/tags/v1", B)
            ]))
        );
        assert_eq!(advertised(b"zzzz"), None);
    }

    #[test]
    fn the_jwt_input_is_two_base64url_parts() {
        let input = jwt_input("123", 1000);
        let (h, c) = input.split_once('.').expect("two parts");
        let claims: Value =
            serde_json::from_slice(&crate::access::base64url(c).expect("b64")).expect("json");
        assert_eq!(claims, json!({ "iat": 940, "exp": 1540, "iss": "123" }));
        let header = crate::access::base64url(h).expect("b64");
        assert_eq!(header, br#"{"alg":"RS256","typ":"JWT"}"#);
        assert_eq!(base64url(b"\xfb\xff"), "-_8");
    }

    #[test]
    fn only_a_pkcs8_key_is_taken() {
        let der = pkcs8_der("-----BEGIN PRIVATE KEY-----\nAAEC\nAw==\n-----END PRIVATE KEY-----\n")
            .expect("der");
        assert_eq!(der, vec![0, 1, 2, 3]);
        let e = pkcs8_der("-----BEGIN RSA PRIVATE KEY-----\nAA\n-----END RSA PRIVATE KEY-----")
            .expect_err("pkcs1");
        assert!(e.contains("openssl pkcs8"), "{e}");
    }

    #[test]
    fn a_github_remote_names_its_repository() {
        assert_eq!(
            github_repo("https://github.com/noisypigeon/pigeon"),
            Some(("noisypigeon".into(), "pigeon".into()))
        );
        assert_eq!(
            github_repo("https://github.com/noisypigeon/pigeon.git"),
            Some(("noisypigeon".into(), "pigeon".into()))
        );
        assert_eq!(github_repo("https://example.com/a/b"), None);
        assert_eq!(
            token_request("pigeon")["permissions"],
            json!({ "contents": "write", "workflows": "write" })
        );
        assert_eq!(
            actions_token_request("pigeon"),
            json!({ "repositories": ["pigeon"], "permissions": { "actions": "write" } }),
            "a dispatch's token can't push"
        );
    }
}
