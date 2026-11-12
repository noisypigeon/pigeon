//! The provider allowlist and lockfile, from their texts; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §5.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

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
    /// The allowlist in `text`, a `providers.yaml`.
    ///
    /// # Errors
    ///
    /// The parser's message when it isn't one.
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_saphyr::from_str(text).map_err(|e| e.to_string())
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

/// The `source` of every provider a HCL file's `required_providers` declares.
pub(crate) fn declared(tf: &str) -> Vec<String> {
    required_blocks(tf)
        .into_iter()
        .flat_map(|b| ENTRY.captures_iter(b))
        .filter_map(|c| {
            let body = c.name("body")?.as_str();
            SOURCE.captures(body).map(|s| s[1].to_string())
        })
        .collect()
}

/// The sources a rendered root `main.tf.json`'s `required_providers` names, and its modules'
/// `source` directories, which the shell reads for theirs.
#[must_use]
pub fn rendered(text: &str) -> (Vec<String>, Vec<String>) {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return (Vec::new(), Vec::new());
    };
    let sources = root["terraform"]["required_providers"]
        .as_object()
        .into_iter()
        .flat_map(|r| r.values())
        .filter_map(|p| p["source"].as_str().map(str::to_string))
        .collect();
    let modules = root["module"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter_map(|m| m["source"].as_str().map(str::to_string))
        .collect();
    (sources, modules)
}

/// The allowlisted sources among `sources`, a unit's declared ones.
#[must_use]
pub fn used_of<'a>(allow: &'a Allowlist, sources: &[String]) -> Vec<&'a str> {
    allow
        .providers
        .keys()
        .filter(|s| {
            sources
                .iter()
                .any(|d| d == *s || d.ends_with(&format!("/{s}")))
        })
        .map(String::as_str)
        .collect()
}

/// The lockfile cut down to the blocks for `sources`.
#[must_use]
pub fn lock_for(lock: &str, sources: &[&str]) -> String {
    let mut out = String::new();
    let mut keep = true;
    for line in lock.lines() {
        if let Some(addr) = line
            .strip_prefix("provider \"")
            .and_then(|r| r.split('"').next())
        {
            keep = sources.iter().any(|s| addr.ends_with(&format!("/{s}")));
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
        if line == "}" {
            keep = true;
        }
    }
    out
}

/// The registry the allowlisted providers come from.
pub const REGISTRY: &str = "registry.opentofu.org";

/// The platform a job's providers are built for.
pub const TARGET: &str = "linux_amd64";

/// The package name of provider `source` at `version`, for [`TARGET`]:
/// `terraform-provider-<type>_<version>_<target>.zip`.
#[must_use]
pub fn package(source: &str, version: &str) -> String {
    format!(
        "terraform-provider-{}_{version}_{TARGET}.zip",
        Allowlist::local_name(source)
    )
}

/// Where a packed filesystem mirror keeps `source`'s package, under the mirror's root.
#[must_use]
pub fn mirror_path(source: &str, version: &str) -> String {
    format!("{REGISTRY}/{source}/{}", package(source, version))
}

/// An `OpenTofu` CLI configuration that installs `sources` from the packed mirror at `mirror`,
/// and every other provider directly; the lock file still checks each package's hash. See
/// `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.
#[must_use]
pub fn mirror_config(mirror: &Path, sources: &[String]) -> String {
    let list = sources
        .iter()
        .map(|s| format!("\"{REGISTRY}/{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "provider_installation {{\n  filesystem_mirror {{\n    path    = \"{}\"\n    include = [{list}]\n  }}\n  direct {{\n    exclude = [{list}]\n  }}\n}}\n",
        mirror.display()
    )
}

/// Where a platform's state bucket keeps the providers' packages.
pub const PACKAGES_PREFIX: &str = "providers/";

#[cfg(test)]
mod tests {
    use super::{Allowlist, declared, lock_for};

    fn allow() -> Allowlist {
        Allowlist {
            description: "x".into(),
            providers: [("cloudflare/cloudflare".to_string(), "5.1.0".to_string())].into(),
        }
    }

    #[test]
    fn only_declarations_count_as_use() {
        let tf = "terraform {\n  required_providers {\n    cf = {\n      source = \"cloudflare/cloudflare\"\n      version = \"5.28.0\"\n    }\n  }\n}\nresource \"terraform_data\" \"x\" { input = \"hashicorp/random\" }\n";
        assert_eq!(declared(tf), ["cloudflare/cloudflare"]);
        assert_eq!(
            declared("locals { x = \"cloudflare/cloudflare\" }\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn lockfiles_keep_only_the_named_providers() {
        let lock = "# header\n\nprovider \"registry.opentofu.org/cloudflare/cloudflare\" {\n  version = \"5.28.0\"\n}\n\nprovider \"registry.opentofu.org/hashicorp/random\" {\n  version = \"3.0.0\"\n}\n";
        let cf = lock_for(lock, &["cloudflare/cloudflare"]);
        assert!(
            cf.contains("cloudflare/cloudflare") && !cf.contains("random"),
            "{cf}"
        );
        assert!(!lock_for(lock, &[]).contains("provider"));
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
