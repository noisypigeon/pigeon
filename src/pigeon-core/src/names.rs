//! The neutral names read beside the old ones, until step 12 drops the old; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §5.

use serde_json::Value;

/// Old and new environment-variable prefixes; the first that matches renames.
const PREFIXES: [(&str, &str); 2] = [
    ("PIGEON_SENTINEL", "PIGEON_EVICTION"),
    ("BACKPLANE_TOKEN", "AGENT_TOKEN"),
];

/// The new name of the variable `old`, or `None` when it keeps its name.
#[must_use]
pub fn renamed(old: &str) -> Option<String> {
    PREFIXES
        .iter()
        .find_map(|(from, to)| old.strip_prefix(from).map(|rest| format!("{to}{rest}")))
}

/// `old` and, first, its new name: every name the variable is read under.
#[must_use]
pub fn both(old: &str) -> Vec<String> {
    renamed(old).into_iter().chain([old.to_string()]).collect()
}

/// The variable `old` from `get`: its new name if set and non-empty, else `old`; with the
/// warning to print when only `old` is set.
pub fn read_with(
    old: &str,
    get: impl Fn(&str) -> Option<String>,
) -> (Option<String>, Option<String>) {
    let Some(new) = renamed(old) else {
        return (get(old), None);
    };
    if let Some(v) = get(&new).filter(|v| !v.is_empty()) {
        return (Some(v), None);
    }
    match get(old) {
        Some(v) => (
            Some(v),
            Some(format!("warning: {old} is deprecated; set {new}")),
        ),
        None => (None, None),
    }
}

/// [`read_with`], printing its warning on stderr once per variable.
pub fn read(old: &str, get: impl Fn(&str) -> Option<String>) -> Option<String> {
    static WARNED: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    let (v, warning) = read_with(old, get);
    if let Some(w) = warning
        && WARNED.lock().is_ok_and(|mut s| s.insert(old.to_string()))
    {
        eprintln!("{w}");
    }
    v
}

/// `get`, with every key read through [`read`].
pub fn lookup(get: impl Fn(&str) -> Option<String>) -> impl Fn(&str) -> Option<String> {
    move |k| read(k, &get)
}

/// The schema extension's key, and the old one read beside it.
pub const SCHEMA_KEY: &str = "x-pigeon";
/// See [`SCHEMA_KEY`].
pub const OLD_SCHEMA_KEY: &str = "x-cobot";

/// `schema`'s extension: `x-pigeon` if present, else `x-cobot` (`Null` when neither is).
#[must_use]
pub fn ext(schema: &Value) -> &Value {
    schema
        .get(SCHEMA_KEY)
        .or_else(|| schema.get(OLD_SCHEMA_KEY))
        .unwrap_or(&Value::Null)
}

/// The ciphertext tags read, the written one first.
pub const ENC_PREFIXES: [&str; 2] = ["ENC[pigeon:v1:", "ENC[cobot:v1:"];

/// The ciphertext body after either tag in [`ENC_PREFIXES`].
#[must_use]
pub fn enc_body(text: &str) -> Option<&str> {
    ENC_PREFIXES.iter().find_map(|p| text.strip_prefix(p))
}

/// Work directories at the root, the written one first; both are git-ignored.
pub const WORK_DIRS: [&str; 2] = [".pigeon", ".cobot"];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn get(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn each_old_prefix_has_its_new_name() {
        for (old, new) in [
            ("BACKPLANE_TOKEN", "AGENT_TOKEN"),
            ("PIGEON_SENTINEL", "PIGEON_EVICTION"),
            ("PIGEON_SENTINEL_URL", "PIGEON_EVICTION_URL"),
            ("PIGEON_SENTINEL_SHA256", "PIGEON_EVICTION_SHA256"),
        ] {
            assert_eq!(renamed(old).as_deref(), Some(new), "{old}");
        }
        for kept in [
            "CLOUDFLARE_API_TOKEN",
            "COBOT_STATE_PASSPHRASE",
            "BACKPLANE_TLS_CERT",
            "PIGEON_STATE_PASSPHRASE",
        ] {
            assert_eq!(renamed(kept), None, "{kept}");
        }
        assert_eq!(both("BACKPLANE_TOKEN"), ["AGENT_TOKEN", "BACKPLANE_TOKEN"]);
    }

    #[test]
    fn the_new_name_wins_over_the_old() {
        let g = get(&[("AGENT_TOKEN", "new"), ("BACKPLANE_TOKEN", "old")]);
        assert_eq!(read_with("BACKPLANE_TOKEN", g), (Some("new".into()), None));
    }

    #[test]
    fn the_old_name_still_reads_with_a_warning() {
        let (v, w) = read_with("BACKPLANE_TOKEN", get(&[("BACKPLANE_TOKEN", "old")]));
        assert_eq!(v.as_deref(), Some("old"));
        assert_eq!(
            w.as_deref(),
            Some("warning: BACKPLANE_TOKEN is deprecated; set AGENT_TOKEN")
        );
    }

    #[test]
    fn an_empty_new_name_falls_back_to_the_old() {
        let g = get(&[("PIGEON_EVICTION", ""), ("PIGEON_SENTINEL", "/bin/judge")]);
        assert_eq!(
            read_with("PIGEON_SENTINEL", g).0.as_deref(),
            Some("/bin/judge")
        );
    }

    #[test]
    fn neither_name_reads_none_and_an_unrenamed_one_reads_itself() {
        assert_eq!(read_with("BACKPLANE_TOKEN", get(&[])), (None, None));
        let g = get(&[("CLOUDFLARE_API_TOKEN", "t")]);
        assert_eq!(
            read_with("CLOUDFLARE_API_TOKEN", g),
            (Some("t".into()), None)
        );
    }

    #[test]
    fn x_pigeon_reads_beside_x_cobot_and_wins() {
        assert_eq!(
            ext(&json!({"x-cobot": {"local": true}}))["local"],
            json!(true)
        );
        assert_eq!(
            ext(&json!({"x-pigeon": {"local": true}}))["local"],
            json!(true)
        );
        let both = json!({"x-pigeon": {"v": "new"}, "x-cobot": {"v": "old"}});
        assert_eq!(ext(&both)["v"], json!("new"));
        assert!(ext(&json!({})).is_null());
    }

    #[test]
    fn both_ciphertext_tags_read() {
        assert_eq!(enc_body("ENC[cobot:v1:abc]"), Some("abc]"));
        assert_eq!(enc_body("ENC[pigeon:v1:abc]"), Some("abc]"));
        assert_eq!(enc_body("ENC[gantry:v1:abc]"), None);
    }
}
