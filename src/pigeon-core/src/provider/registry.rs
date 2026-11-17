//! The registry of MCP servers a call may reach, with their pinned tools.

use std::collections::BTreeMap;

use regex::Regex;
use serde::Deserialize;

/// The committed `.config/providers/mcp.yaml`.
pub const CONFIG: &str = include_str!("../../../../.config/providers/mcp.yaml");

/// The registry: every server, and what is exempt from the broker.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// Servers by name.
    pub servers: BTreeMap<String, Server>,
    /// Providers exempt from the broker.
    #[serde(default)]
    pub exempt: Vec<Exempt>,
}

/// One MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// `https://` URL, or `inproc:<name>`.
    pub url: String,
    /// The credential kind, as `Credential::parse` reads it.
    pub credential: String,
    /// Fixed non-credential request headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Pinned tools by name.
    pub tools: BTreeMap<String, Tool>,
}

/// One pinned tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    /// What the tool does to the provider.
    pub effect: Effect,
    /// The capability a caller needs.
    pub capability: String,
    /// The credential's scopes.
    #[serde(default)]
    pub scope: Vec<String>,
    /// The 64-hex SHA-256 of the tool's `inputSchema`.
    pub schema: String,
}

/// A tool's effect, by increasing tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Reads only.
    Read,
    /// Changes provider state.
    Write,
    /// Applies a change that needs the owner's proof.
    Apply,
}

impl Effect {
    /// The lowercase word for the effect.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Apply => "apply",
        }
    }
}

/// A provider left out of the broker, and why.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exempt {
    /// Its identifier.
    pub id: String,
    /// What is exempt.
    pub what: String,
    /// Why it is exempt.
    pub why: String,
    /// Where the reason is shown.
    pub evidence: Vec<String>,
}

/// Where a server's credential comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// A GitHub App installation token.
    GithubInstallation,
    /// A Cloudflare token minted per call.
    CloudflareMinted,
    /// An App Store Connect JWT.
    AscJwt,
    /// A named platform secret.
    Secret(String),
}

impl Credential {
    /// The credential `text` names, or None.
    ///
    /// # Panics
    /// Never: the pattern is static.
    #[must_use]
    pub fn parse(text: &str) -> Option<Credential> {
        match text {
            "github-installation" => Some(Self::GithubInstallation),
            "cloudflare-minted" => Some(Self::CloudflareMinted),
            "asc-jwt" => Some(Self::AscJwt),
            _ => {
                let name = text.strip_prefix("secret:")?;
                let ok = Regex::new(r"^[A-Z][A-Z0-9_]*$").expect("static pattern");
                ok.is_match(name).then(|| Self::Secret(name.to_string()))
            }
        }
    }
}

/// Parses and validates the registry YAML, listing every problem.
///
/// # Panics
/// Never: the patterns are static.
///
/// # Errors
/// Every problem found, in rule order.
pub fn parse(yaml: &str) -> Result<Registry, Vec<String>> {
    let registry: Registry = serde_saphyr::from_str(yaml).map_err(|e| vec![e.to_string()])?;
    let server_name = Regex::new(r"^[a-z][a-z0-9-]*$").expect("static pattern");
    let tool_name = Regex::new(r"^[a-z][a-z0-9_]*$").expect("static pattern");
    let hex64 = Regex::new(r"^[0-9a-f]{64}$").expect("static pattern");
    let mut problems = Vec::new();
    let each = |f: &mut dyn FnMut(&String, &Server)| {
        for (n, s) in &registry.servers {
            f(n, s);
        }
    };
    each(&mut |n, _| {
        if !server_name.is_match(n) {
            problems.push(format!(
                "server `{n}`: a name is lowercase letters, digits and dashes"
            ));
        }
    });
    each(&mut |n, s| {
        let inproc = s
            .url
            .strip_prefix("inproc:")
            .is_some_and(|x| server_name.is_match(x));
        if !s.url.starts_with("https://") && !inproc {
            problems.push(format!("server `{n}`: url is https:// or inproc:<name>"));
        }
    });
    each(&mut |n, s| {
        if Credential::parse(&s.credential).is_none() {
            problems.push(format!(
                "server `{n}`: credential `{}` is not github-installation, cloudflare-minted, asc-jwt or secret:NAME",
                s.credential
            ));
        }
    });
    each(&mut |n, s| {
        for h in s.headers.keys() {
            let l = h.to_lowercase();
            let bad = ["authorization", "cookie", "proxy-authorization"].contains(&l.as_str())
                || ["token", "secret", "apikey", "api-key"]
                    .iter()
                    .any(|w| l.contains(w));
            if bad {
                problems.push(format!(
                    "server `{n}`: header `{h}` would carry a credential; credentials come from `credential` only"
                ));
            }
        }
    });
    each(&mut |n, s| {
        if s.tools.is_empty() {
            problems.push(format!("server `{n}`: lists no tools"));
        }
    });
    each(&mut |n, s| {
        for t in s.tools.keys() {
            if !tool_name.is_match(t) {
                problems.push(format!(
                    "server `{n}` tool `{t}`: a tool name is lowercase letters, digits and underscores"
                ));
            }
        }
    });
    each(&mut |n, s| {
        for (t, tool) in &s.tools {
            if !hex64.is_match(&tool.schema) {
                problems.push(format!(
                    "server `{n}` tool `{t}`: schema is the 64-hex SHA-256 of its inputSchema"
                ));
            }
        }
    });
    each(&mut |n, s| {
        for (t, tool) in &s.tools {
            if tool.capability.is_empty() {
                problems.push(format!("server `{n}` tool `{t}`: names no capability"));
            }
        }
    });
    let mut seen = std::collections::BTreeSet::new();
    for e in &registry.exempt {
        if !seen.insert(&e.id) {
            problems.push(format!("exempt `{}`: listed twice", e.id));
        }
    }
    for e in &registry.exempt {
        if e.what.is_empty() || e.why.is_empty() || e.evidence.is_empty() {
            problems.push(format!("exempt `{}`: needs what, why and evidence", e.id));
        }
    }
    if problems.is_empty() {
        Ok(registry)
    } else {
        Err(problems)
    }
}

impl Registry {
    /// The server and tool a call names.
    #[must_use]
    pub fn tool(&self, server: &str, tool: &str) -> Option<(&Server, &Tool)> {
        let s = self.servers.get(server)?;
        Some((s, s.tools.get(tool)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn the_committed_registry_parses() {
        let r = parse(CONFIG).expect("committed registry");
        assert!(r.servers.is_empty());
        assert_eq!(r.exempt.len(), 10);
    }

    #[test]
    fn every_exemption_names_code_evidence() {
        let r = parse(CONFIG).expect("committed registry");
        let roots = [
            "src/",
            "tooling/",
            "infra/",
            "catalog/",
            "Dockerfile",
            ".github/",
        ];
        for e in &r.exempt {
            for path in &e.evidence {
                assert!(
                    roots.iter().any(|root| path.starts_with(root)),
                    "{}: {path}",
                    e.id
                );
            }
        }
    }

    #[test]
    fn a_registry_parses() {
        let yaml = format!(
            "servers:\n  buildkite:\n    url: https://mcp.buildkite.com/mcp\n    credential: secret:BUILDKITE_TOKEN\n    headers:\n      X-Buildkite-Toolsets: builds,pipelines\n    tools:\n      get_build:\n        effect: read\n        capability: ci.read\n        scope: [read_builds]\n        schema: {A}\nexempt:\n  - id: git\n    what: the git door\n    why: it is not MCP\n    evidence: [docs]\n"
        );
        let r = parse(&yaml).expect("parses");
        let s = &r.servers["buildkite"];
        assert_eq!(s.url, "https://mcp.buildkite.com/mcp");
        assert_eq!(s.credential, "secret:BUILDKITE_TOKEN");
        assert_eq!(s.headers["X-Buildkite-Toolsets"], "builds,pipelines");
        let t = &s.tools["get_build"];
        assert_eq!(t.effect, Effect::Read);
        assert_eq!(t.capability, "ci.read");
        assert_eq!(t.scope, vec!["read_builds".to_string()]);
        assert_eq!(t.schema, A);
        assert_eq!(r.exempt.len(), 1);
        assert_eq!(r.exempt[0].id, "git");
        assert_eq!(r.exempt[0].evidence, vec!["docs".to_string()]);
        assert!(r.tool("buildkite", "get_build").is_some());
        assert!(r.tool("buildkite", "nope").is_none());
    }

    #[test]
    fn inproc_and_every_credential_kind_parse() {
        let tool = format!(
            "tools:\n      t:\n        effect: write\n        capability: c\n        schema: {A}\n"
        );
        let yaml = format!(
            "servers:\n  asc:\n    url: inproc:asc\n    credential: asc-jwt\n    {tool}  gh:\n    url: https://a.example/mcp\n    credential: github-installation\n    {tool}  cf:\n    url: https://b.example/mcp\n    credential: cloudflare-minted\n    {tool}  sec:\n    url: https://c.example/mcp\n    credential: secret:A_1\n    {tool}"
        );
        let r = parse(&yaml).expect("parses");
        assert_eq!(r.servers.len(), 4);
        assert_eq!(
            Credential::parse("secret:A_1"),
            Some(Credential::Secret("A_1".into()))
        );
        assert_eq!(Credential::parse("asc-jwt"), Some(Credential::AscJwt));
        assert_eq!(
            Credential::parse("github-installation"),
            Some(Credential::GithubInstallation)
        );
        assert_eq!(
            Credential::parse("cloudflare-minted"),
            Some(Credential::CloudflareMinted)
        );
    }

    #[test]
    fn credential_parse_refuses() {
        for bad in ["secret:lower", "secret:", "oauth", ""] {
            assert_eq!(Credential::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn every_problem_is_listed() {
        let yaml = "servers:\n  Bad_Name:\n    url: http://x\n    credential: oauth\n    headers:\n      Authorization: x\n    tools:\n      Bad-Tool:\n        effect: read\n        capability: ''\n        schema: xyz\n  empty:\n    url: https://e.example\n    credential: asc-jwt\n    tools: {}\nexempt:\n  - id: d\n    what: ''\n    why: w\n    evidence: [e]\n  - id: d\n    what: w\n    why: w\n    evidence: [e]\n";
        let p = parse(yaml).expect_err("problems");
        let words = [
            "a name is lowercase",
            "url is https://",
            "is not github-installation",
            "would carry a credential",
            "lists no tools",
            "a tool name is lowercase",
            "schema is the 64-hex",
            "names no capability",
            "listed twice",
            "needs what, why and evidence",
        ];
        assert_eq!(p.len(), 10, "{p:?}");
        for (m, w) in p.iter().zip(words) {
            assert!(m.contains(w), "{m} lacks {w}");
        }
    }

    #[test]
    fn headers_that_carry_credentials_are_refused() {
        let mk = |h: &str| {
            format!(
                "servers:\n  s:\n    url: https://s.example\n    credential: asc-jwt\n    headers:\n      {h}: x\n    tools:\n      t:\n        effect: read\n        capability: c\n        schema: {A}\n"
            )
        };
        for bad in ["Authorization", "X-Api-Key", "X-Auth-Token", "cookie"] {
            assert!(parse(&mk(bad)).is_err(), "{bad}");
        }
        for ok in ["X-Buildkite-Toolsets", "X-MCP-Toolsets"] {
            assert!(parse(&mk(ok)).is_ok(), "{ok}");
        }
    }
}
