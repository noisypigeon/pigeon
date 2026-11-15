//! The learner's practice on an instance: forms, scored attempts and progress, each sealed in the
//! instance's bucket, and the index the `Instance` keeps for them; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md` §5 and its
//! 2026-11-15 amendment.

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead as _, KeyInit as _};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[cfg(test)]
mod tests;

/// The sample form's number; it is never kept.
pub const SAMPLE_FORM: u32 = 1023;

/// Questions per printed page.
pub const PER_PAGE: usize = 25;

/// Where the `Instance` keeps the index.
pub const INDEX: &str = "learner.index";

/// The HKDF info the sealing key is derived with.
pub const INFO: &[u8] = b"pigeon-learner-v1";

/// The bucket's prefix for the learner's objects.
pub const PREFIX: &str = "learner/";

/// The most items one sync takes.
pub const MAX_ITEMS: usize = 2000;

/// A circle's centre on a printed page, in millimetres from its top-left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Circle {
    /// The question's index on the form.
    pub question: u32,
    /// 0 for A to 3 for D.
    pub choice: u8,
    /// Across.
    pub x: f64,
    /// Down.
    pub y: f64,
}

/// A printed form: its number, questions in order, key, and each page's circles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Form {
    /// Its number, 0 to 1022.
    pub number: u32,
    /// The question IDs in order.
    #[serde(rename = "questionIDs")]
    pub question_ids: Vec<u32>,
    /// Each question's correct choice, 0 to 3.
    pub key: Vec<u8>,
    /// Each page's circles, as laid out when it printed.
    #[serde(default)]
    pub circles: Vec<Vec<Circle>>,
}

/// How one question was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkResult {
    /// One mark, on the key.
    Correct,
    /// One mark, elsewhere.
    Wrong,
    /// Two or more marks, scored as wrong.
    Double,
    /// No mark.
    Blank,
}

/// One scored page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    /// Its ID.
    pub id: String,
    /// Its form.
    pub form: u32,
    /// Its page, 0 to 3.
    pub page: u32,
    /// When, in Unix seconds.
    pub date: i64,
    /// The page's question IDs.
    #[serde(rename = "questionIDs")]
    pub question_ids: Vec<u32>,
    /// Each question's filled circles.
    pub marks: Vec<Vec<u8>>,
    /// Each question's result, as the instance scored it.
    pub results: Vec<MarkResult>,
}

impl Attempt {
    /// Its right answers.
    #[must_use]
    pub fn correct(&self) -> usize {
        self.results
            .iter()
            .filter(|r| **r == MarkResult::Correct)
            .count()
    }
}

/// An attempt as the index lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The attempt's ID.
    pub id: String,
    /// Its form.
    pub form: u32,
    /// Its page.
    pub page: u32,
    /// When.
    pub date: i64,
    /// Right answers.
    pub correct: usize,
    /// Questions.
    pub count: usize,
    /// Its content's SHA-256, hex.
    pub sha: String,
}

/// What the `Instance` keeps: every form's content hash, and every attempt's entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// Each held form's content hash, by number.
    #[serde(default)]
    pub forms: BTreeMap<u32, String>,
    /// Each held attempt, oldest first.
    #[serde(default)]
    pub attempts: Vec<Entry>,
}

impl Index {
    /// The index of these forms and attempts, as a rebuild from the bucket finds them.
    #[must_use]
    pub fn of(forms: &[Form], attempts: &[Attempt]) -> Self {
        let mut ix = Self::default();
        for f in forms {
            ix.forms.insert(f.number, sha(f));
        }
        for a in attempts {
            ix.attempts.push(entry(a));
        }
        ix.attempts
            .sort_by(|a, b| (a.date, &a.id).cmp(&(b.date, &b.id)));
        ix
    }

    /// Whether attempt `id` is held.
    #[must_use]
    pub fn holds(&self, id: &str) -> Option<&Entry> {
        self.attempts.iter().find(|e| e.id == id)
    }
}

/// One question's record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    /// Times seen.
    pub seen: u32,
    /// Times right.
    pub right: u32,
    /// Times not right: wrong, double or blank.
    pub wrong: u32,
    /// When last seen, in Unix seconds.
    pub last: i64,
}

/// Each question's record, by ID.
pub type Progress = BTreeMap<u32, Tally>;

/// `progress` with `attempt` counted in.
pub fn tally(progress: &mut Progress, attempt: &Attempt) {
    for (id, r) in attempt.question_ids.iter().zip(&attempt.results) {
        let t = progress.entry(*id).or_default();
        t.seen += 1;
        if *r == MarkResult::Correct {
            t.right += 1;
        } else {
            t.wrong += 1;
        }
        t.last = t.last.max(attempt.date);
    }
}

/// The progress of every attempt.
#[must_use]
pub fn progress_of(attempts: &[Attempt]) -> Progress {
    let mut p = Progress::new();
    for a in attempts {
        tally(&mut p, a);
    }
    p
}

/// Each question's result: one mark on the key is correct, one elsewhere wrong, two or more
/// double, none blank. As the app's `score`.
#[must_use]
pub fn score(key: &[u8], marks: &[Vec<u8>]) -> Vec<MarkResult> {
    key.iter()
        .enumerate()
        .map(|(i, k)| match marks.get(i).map_or(&[][..], Vec::as_slice) {
            [] => MarkResult::Blank,
            [only] if only == k => MarkResult::Correct,
            [_] => MarkResult::Wrong,
            _ => MarkResult::Double,
        })
        .collect()
}

/// Page `page`'s question IDs and key.
#[must_use]
pub fn page_slice(form: &Form, page: u32) -> (&[u32], &[u8]) {
    let n = form.question_ids.len().min(form.key.len());
    let start = (page as usize).saturating_mul(PER_PAGE).min(n);
    let end = (start + PER_PAGE).min(n);
    (&form.question_ids[start..end], &form.key[start..end])
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn sha<T: Serialize>(v: &T) -> String {
    hex(&Sha256::digest(serde_json::to_vec(v).unwrap_or_default()))
}

fn entry(a: &Attempt) -> Entry {
    Entry {
        id: a.id.clone(),
        form: a.form,
        page: a.page,
        date: a.date,
        correct: a.correct(),
        count: a.results.len(),
        sha: sha(a),
    }
}

/// The bucket key of form `n`.
#[must_use]
pub fn form_key(n: u32) -> String {
    format!("{PREFIX}forms/{n}.json")
}

/// The bucket key of attempt `id`.
#[must_use]
pub fn attempt_key(id: &str) -> String {
    format!("{PREFIX}attempts/{id}.json")
}

/// The bucket key of the progress record.
#[must_use]
pub fn progress_key() -> String {
    format!("{PREFIX}progress.json")
}

fn check_form(f: &Form) -> Result<(), String> {
    if f.number >= SAMPLE_FORM {
        return Err(format!("form {}: not a kept form's number", f.number));
    }
    if f.question_ids.is_empty() || f.question_ids.len() != f.key.len() {
        return Err(format!(
            "form {}: its questions and key differ in length",
            f.number
        ));
    }
    if f.question_ids.len() > 4 * PER_PAGE {
        return Err(format!("form {}: more than four pages", f.number));
    }
    if f.key.iter().any(|k| *k > 3) {
        return Err(format!("form {}: a key outside A to D", f.number));
    }
    Ok(())
}

fn check_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(format!("attempt `{id}`: not an attempt ID"))
    }
}

/// What a sync sends.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sync {
    /// Forms.
    #[serde(default)]
    pub forms: Vec<Form>,
    /// Attempts.
    #[serde(default)]
    pub attempts: Vec<Attempt>,
}

/// What a sync does: what to write, the next index, and what it refused, by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Planned {
    /// Forms to write.
    pub forms: Vec<Form>,
    /// Attempts to write, scored by the instance.
    pub attempts: Vec<Attempt>,
    /// The index once they are written.
    pub index: Index,
    /// Each item refused, and why.
    pub refused: Vec<String>,
}

/// Plans `sync` against `index`. `held` are stored forms its attempts name that it doesn't send.
/// An item already held with the same content is accepted again and not rewritten; one held
/// with other content is refused by name. Attempts are scored here; the sender's results are
/// not used.
#[must_use]
pub fn plan(index: &Index, sync: Sync, held: &BTreeMap<u32, Form>) -> Planned {
    let mut out = Planned {
        index: index.clone(),
        ..Planned::default()
    };
    if sync.forms.len() + sync.attempts.len() > MAX_ITEMS {
        out.refused
            .push(format!("at most {MAX_ITEMS} items are synced at once"));
        return out;
    }
    let mut known = held.clone();
    for f in sync.forms {
        if let Err(why) = check_form(&f) {
            out.refused.push(why);
            continue;
        }
        let s = sha(&f);
        match out.index.forms.get(&f.number) {
            Some(h) if *h == s => {}
            Some(_) => {
                out.refused
                    .push(format!("form {}: held with other content", f.number));
                continue;
            }
            None => {
                out.index.forms.insert(f.number, s);
                out.forms.push(f.clone());
            }
        }
        known.insert(f.number, f);
    }
    for a in sync.attempts {
        if let Err(why) = check_id(&a.id) {
            out.refused.push(why);
            continue;
        }
        let Some(form) = known.get(&a.form) else {
            out.refused
                .push(format!("attempt {}: form {} is not held", a.id, a.form));
            continue;
        };
        let (ids, key) = page_slice(form, a.page);
        if ids.is_empty() || ids != a.question_ids.as_slice() {
            out.refused.push(format!(
                "attempt {}: its questions aren't page {} of form {}",
                a.id, a.page, a.form
            ));
            continue;
        }
        let scored = Attempt {
            results: score(key, &a.marks),
            ..a
        };
        let e = entry(&scored);
        match out.index.holds(&scored.id) {
            Some(h) if h.sha == e.sha => {}
            Some(_) => out
                .refused
                .push(format!("attempt {}: held with other content", scored.id)),
            None => {
                out.index.attempts.push(e);
                out.attempts.push(scored);
            }
        }
    }
    out.index
        .attempts
        .sort_by(|a, b| (a.date, &a.id).cmp(&(b.date, &b.id)));
    out
}

/// What a sync answers: everything the instance holds, so the sender can let go of exactly
/// those, and what it refused.
#[must_use]
pub fn answer(index: &Index, refused: &[String]) -> serde_json::Value {
    serde_json::json!({
        "forms": index.forms.keys().collect::<Vec<_>>(),
        "attempts": index.attempts.iter().map(|e| &e.id).collect::<Vec<_>>(),
        "refused": refused,
    })
}

/// The sealing key: HKDF-SHA256 of `STATE_KEY`'s 32 bytes (hex), with [`INFO`].
///
/// # Errors
/// `state_key` isn't 64 hex characters.
pub fn sealing_key(state_key: &str) -> Result<[u8; 32], String> {
    let t = state_key.trim();
    if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("STATE_KEY is not 32 bytes of hex".into());
    }
    let ikm: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).unwrap_or_default())
        .collect();
    let mut okm = [0u8; 32];
    hkdf::Hkdf::<Sha256>::new(None, &ikm)
        .expand(INFO, &mut okm)
        .map_err(|e| e.to_string())?;
    Ok(okm)
}

/// `plain` sealed with AES-256-GCM under `key`: the 12-byte nonce, then the ciphertext.
///
/// # Errors
/// When encryption fails.
pub fn seal(key: &[u8; 32], nonce: [u8; 12], plain: &[u8]) -> Result<Vec<u8>, String> {
    let c = Aes256Gcm::new(key.into());
    let mut out = nonce.to_vec();
    out.extend(
        c.encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| "sealing failed".to_string())?,
    );
    Ok(out)
}

/// What [`seal`] sealed.
///
/// # Errors
/// Too short, or the tag doesn't verify under `key`.
pub fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.len() < 12 + 16 {
        return Err("too short to be sealed".into());
    }
    let (nonce, body) = sealed.split_at(12);
    Aes256Gcm::new(key.into())
        .decrypt(Nonce::from_slice(nonce), body)
        .map_err(|_| "it doesn't open under this key".into())
}
