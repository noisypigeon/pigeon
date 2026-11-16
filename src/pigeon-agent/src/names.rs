//! The neutral names read beside the old ones, from core, and their reader of the process
//! environment; see `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §5.

pub use pigeon_core::names::*;

/// [`read`] from the process environment.
#[must_use]
pub fn env(old: &str) -> Option<String> {
    read(old, |k| std::env::var(k).ok())
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
    fn r2_state_reads_only_pigeon_variables() {
        use crate::cobot::state::Store;
        for (prefix, reads) in [("PIGEON_", true), ("COBOT_", false)] {
            let vars = r2(prefix);
            let pairs: Vec<(&str, &str)> =
                vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let store = Store::parse("r2://bucket/units", get(&pairs));
            assert_eq!(
                matches!(store, Ok(Store::R2 { .. })),
                reads,
                "{prefix}: {store:?}"
            );
        }
    }

    #[test]
    fn tls_reads_only_agent_variables() {
        let only_new = crate::serve::Tls::from_vars(get(&[("AGENT_TLS_CERT", "pem")]));
        assert!(
            only_new.is_err(),
            "AGENT_TLS_CERT alone must be refused as partial"
        );
        let only_old = crate::serve::Tls::from_vars(get(&[("BACKPLANE_TLS_CERT", "pem")]));
        assert!(
            matches!(only_old, Ok(None)),
            "BACKPLANE_TLS_CERT is no longer read"
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
    fn tools_read_only_pigeon_variables() {
        let t = crate::cobot::workers::Tools::from(get(&[
            ("PIGEON_WRANGLER", "/new/wrangler"),
            ("COBOT_DOCKER", "/old/docker"),
        ]));
        assert_eq!(t.wrangler, Path::new("/new/wrangler"));
        assert_ne!(t.docker, Path::new("/old/docker"));
    }

    #[test]
    fn the_work_directory_is_git_ignored() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join(".config/approvals.yaml").is_file())
            .expect("the repository root");
        let ignore = std::fs::read_to_string(root.join(".gitignore")).expect(".gitignore");
        let dir = format!("/{WORK_DIR}/");
        assert!(ignore.lines().any(|l| l == dir), "{dir}");
    }
}
