//! The provider allowlist and lockfile; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §5.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use crate::Error;

/// The allowlist file inside the providers directory.
pub const ALLOWLIST: &str = "providers.yaml";
/// The lockfile inside the providers directory.
pub const LOCKFILE: &str = "providers.lock.hcl";

static BLOCK_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\brequired_providers\s*\{").expect("valid regex"));
static ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)\b(?<name>[A-Za-z0-9_-]+)\s*=\s*(?:\{(?<body>[^{}]*)\}|"(?<short>[^"]*)")"#)
        .expect("valid regex")
});
static SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bsource\s*=\s*"([^"]+)""#).expect("valid regex"));
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bversion\s*=\s*"([^"]+)""#).expect("valid regex"));

/// The bodies of every `required_providers { … }` block, braces matched.
fn required_blocks(tf: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for m in BLOCK_START.find_iter(tf) {
        let start = m.end();
        let mut depth = 1usize;
        for (i, c) in tf[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(&tf[start..start + i]);
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// `.config/providers.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowlist {
    /// One line saying what the file is.
    pub description: String,
    /// Provider source to exact version.
    #[serde(default)]
    pub providers: BTreeMap<String, String>,
}

impl Allowlist {
    /// Loads `<dir>/providers.yaml`; a missing file allows nothing.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the file exists but is unreadable or invalid.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(ALLOWLIST);
        if !path.exists() {
            return Ok(Self::default());
        }
        let bad = |message: String| Error::Io {
            path: path.clone(),
            message,
        };
        let text = fs::read_to_string(&path).map_err(|e| bad(e.to_string()))?;
        serde_saphyr::from_str(&text).map_err(|e| bad(e.to_string()))
    }

    /// The local name for a source: its last path segment.
    #[must_use]
    pub fn local_name(source: &str) -> &str {
        source.rsplit('/').next().unwrap_or(source)
    }

    /// The allowlisted source whose local name is `name`.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<(&str, &str)> {
        self.providers
            .iter()
            .find(|(s, _)| Self::local_name(s) == name)
            .map(|(s, v)| (s.as_str(), v.as_str()))
    }

    /// Problems with the `required_providers` declared in `tf`.
    #[must_use]
    pub fn check(&self, tf: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for c in required_blocks(tf)
            .into_iter()
            .flat_map(|b| ENTRY.captures_iter(b))
        {
            let name = &c["name"];
            let Some(body) = c.name("body").map(|b| b.as_str()) else {
                problems.push(format!(
                    "provider `{name}` must declare `source` and `version`"
                ));
                continue;
            };
            let (Some(source), Some(version)) = (SOURCE.captures(body), VERSION.captures(body))
            else {
                problems.push(format!(
                    "provider `{name}` must declare both `source` and `version`"
                ));
                continue;
            };
            let (source, version) = (&source[1], &version[1]);
            match self.providers.get(source) {
                None => problems.push(format!("provider `{source}` is not in the allowlist")),
                Some(pinned) if pinned != version.trim_start_matches(['=', ' ']) => {
                    problems.push(format!(
                        "provider `{source}` is `{version}`; the allowlist pins `{pinned}`"
                    ));
                }
                Some(_) => {}
            }
        }
        problems
    }
}

/// The lockfile path, if the providers directory has one.
#[must_use]
pub fn lockfile(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(LOCKFILE);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::Allowlist;

    fn allow() -> Allowlist {
        Allowlist {
            description: "x".into(),
            providers: [("cloudflare/cloudflare".to_string(), "5.1.0".to_string())].into(),
        }
    }

    #[test]
    fn checks_sources_and_exact_versions() {
        let ok = "terraform {\n  required_providers {\n    cloudflare = {\n      source  = \"cloudflare/cloudflare\"\n      version = \"5.1.0\"\n    }\n  }\n}\n";
        assert_eq!(allow().check(ok), Vec::<String>::new());
        assert_eq!(
            allow().check(&ok.replace("5.1.0", "= 5.1.0")),
            Vec::<String>::new()
        );
        assert_eq!(allow().check(&ok.replace("5.1.0", "5.2.0")).len(), 1);
        assert_eq!(
            allow()
                .check(&ok.replace("cloudflare/cloudflare", "evil/cloudflare"))
                .len(),
            1
        );
        let reordered = "terraform {\n  required_providers {\n    cf = {\n      version = \"5.1.0\"\n      source = \"evil/cloudflare\"\n    }\n  }\n}\n";
        assert_eq!(allow().check(reordered).len(), 1);
        let unpinned = "terraform {\n  required_providers {\n    cf = { source = \"cloudflare/cloudflare\" }\n  }\n}\n";
        assert_eq!(allow().check(unpinned).len(), 1);
        let shorthand = "terraform {\n  required_providers {\n    evil = \"1.0.0\"\n  }\n}\n";
        assert_eq!(allow().check(shorthand).len(), 1);
        assert_eq!(
            allow().by_name("cloudflare"),
            Some(("cloudflare/cloudflare", "5.1.0"))
        );
    }
}
