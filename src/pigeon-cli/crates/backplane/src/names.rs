//! The neutral names read beside the old ones, until step 12 drops the old; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §5.

use serde_json::Value;

/// Old and new environment-variable prefixes; the first that matches renames.
const PREFIXES: [(&str, &str); 3] = [
    ("PIGEON_SENTINEL", "PIGEON_EVICTION"),
    ("COBOT_", "PIGEON_"),
    ("BACKPLANE_", "AGENT_"),
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

/// [`read`] from the process environment.
#[must_use]
pub fn env(old: &str) -> Option<String> {
    read(old, |k| std::env::var(k).ok())
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
pub const ENC_PREFIXES: [&str; 2] = ["ENC[cobot:v1:", "ENC[pigeon:v1:"];

/// The ciphertext body after either tag in [`ENC_PREFIXES`].
#[must_use]
pub fn enc_body(text: &str) -> Option<&str> {
    ENC_PREFIXES.iter().find_map(|p| text.strip_prefix(p))
}

/// Work directories at the root, the written one first; both are git-ignored.
pub const WORK_DIRS: [&str; 2] = [".cobot", ".pigeon"];

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
            ("COBOT_STATE_PASSPHRASE", "PIGEON_STATE_PASSPHRASE"),
            ("COBOT_SECRETS_PASSPHRASE", "PIGEON_SECRETS_PASSPHRASE"),
            (
                "COBOT_WORKLOAD_SECRETS_PASSPHRASE",
                "PIGEON_WORKLOAD_SECRETS_PASSPHRASE",
            ),
            ("COBOT_STATE_R2_ENDPOINT", "PIGEON_STATE_R2_ENDPOINT"),
            ("COBOT_CREDENTIALS", "PIGEON_CREDENTIALS"),
            ("COBOT_DOCKER", "PIGEON_DOCKER"),
            ("COBOT_WRANGLER", "PIGEON_WRANGLER"),
            ("BACKPLANE_TOKEN", "AGENT_TOKEN"),
            ("BACKPLANE_TLS_CLIENT_CA", "AGENT_TLS_CLIENT_CA"),
            ("PIGEON_SENTINEL", "PIGEON_EVICTION"),
            ("PIGEON_SENTINEL_URL", "PIGEON_EVICTION_URL"),
            ("PIGEON_SENTINEL_SHA256", "PIGEON_EVICTION_SHA256"),
        ] {
            assert_eq!(renamed(old).as_deref(), Some(new), "{old}");
        }
        assert_eq!(renamed("CLOUDFLARE_API_TOKEN"), None);
        assert_eq!(both("BACKPLANE_TOKEN"), ["AGENT_TOKEN", "BACKPLANE_TOKEN"]);
    }

    #[test]
    fn the_new_name_wins_over_the_old() {
        let g = get(&[("AGENT_TOKEN", "new"), ("BACKPLANE_TOKEN", "old")]);
        assert_eq!(read_with("BACKPLANE_TOKEN", g), (Some("new".into()), None));
    }

    #[test]
    fn the_old_name_still_reads_with_a_warning() {
        let (v, w) = read_with(
            "COBOT_STATE_PASSPHRASE",
            get(&[("COBOT_STATE_PASSPHRASE", "old")]),
        );
        assert_eq!(v.as_deref(), Some("old"));
        assert_eq!(
            w.as_deref(),
            Some("warning: COBOT_STATE_PASSPHRASE is deprecated; set PIGEON_STATE_PASSPHRASE")
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
        assert_eq!(read_with("COBOT_DOCKER", get(&[])), (None, None));
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

/// Each reader of a renamed variable, given only the new name, or only the old.
#[cfg(all(test, feature = "engines"))]
mod readers {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn get(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn r2(prefix: &str) -> Vec<(String, String)> {
        [
            ("STATE_R2_ENDPOINT", "https://r2.example"),
            ("STATE_R2_ACCESS_KEY_ID", "key-id"),
            ("STATE_R2_SECRET_ACCESS_KEY", "secret"),
        ]
        .iter()
        .map(|(k, v)| (format!("{prefix}{k}"), (*v).to_string()))
        .collect()
    }

    #[test]
    fn r2_state_reads_pigeon_or_cobot_variables() {
        use crate::cobot::state::Store;
        for prefix in ["PIGEON_", "COBOT_"] {
            let vars = r2(prefix);
            let pairs: Vec<(&str, &str)> =
                vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let store = Store::parse("r2://bucket/units", get(&pairs));
            assert!(matches!(store, Ok(Store::R2 { .. })), "{prefix}: {store:?}");
        }
    }

    #[test]
    fn tls_reads_agent_variables() {
        let only_new = crate::serve::Tls::from_vars(get(&[("AGENT_TLS_CERT", "pem")]));
        assert!(
            only_new.is_err(),
            "AGENT_TLS_CERT alone must be refused as partial"
        );
        let only_old = crate::serve::Tls::from_vars(get(&[("BACKPLANE_TLS_CERT", "pem")]));
        assert!(
            only_old.is_err(),
            "BACKPLANE_TLS_CERT alone must be refused as partial"
        );
        assert!(matches!(crate::serve::Tls::from_vars(get(&[])), Ok(None)));
    }

    #[test]
    fn the_judge_is_found_under_pigeon_eviction_or_pigeon_sentinel() {
        let new = get(&[
            ("PIGEON_EVICTION", "/new/judge"),
            ("PIGEON_SENTINEL", "/old/judge"),
        ]);
        assert_eq!(crate::sentinel::locate(&new), PathBuf::from("/new/judge"));
        let old = get(&[("PIGEON_SENTINEL", "/old/judge")]);
        assert_eq!(crate::sentinel::locate(&old), PathBuf::from("/old/judge"));
    }

    #[test]
    fn tools_read_pigeon_or_cobot_variables() {
        let t = crate::cobot::workers::Tools::from(get(&[
            ("PIGEON_WRANGLER", "/new/wrangler"),
            ("COBOT_DOCKER", "/old/docker"),
        ]));
        assert_eq!(t.wrangler, Path::new("/new/wrangler"));
        assert_eq!(t.docker, Path::new("/old/docker"));
    }

    #[test]
    fn both_work_directories_are_git_ignored() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join(".config/approvals.yaml").is_file())
            .expect("the repository root");
        let ignore = std::fs::read_to_string(root.join(".gitignore")).expect(".gitignore");
        for d in WORK_DIRS {
            assert!(ignore.lines().any(|l| l == format!("/{d}/")), "{d}");
        }
    }
}
