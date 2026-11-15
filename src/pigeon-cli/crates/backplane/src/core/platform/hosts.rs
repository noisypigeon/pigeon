//! Standalone hostnames under the platform's zone: labels, which host is whose, and what the
//! proxy passes; see `.claude/decisions/2026-11-15-runtime-standalone-hostnames-platform-zone.md`.

/// Labels the platform keeps for itself (§2).
pub const RESERVED: [&str; 29] = [
    "platform",
    "www",
    "api",
    "app",
    "blue",
    "green",
    "git",
    "docs",
    "control",
    "rescue",
    "users",
    "mail",
    "mx",
    "smtp",
    "imap",
    "autoconfig",
    "autodiscover",
    "admin",
    "auth",
    "login",
    "id",
    "status",
    "static",
    "cdn",
    "assets",
    "ns1",
    "ns2",
    "pigeon",
    "noisypigeon",
];

/// The only paths a standalone's host serves.
pub const PATH_PREFIX: &str = "/v0/app/";

/// Request headers the proxy passes, besides every `X-Pigeon-*` header.
pub const PASSED: [&str; 3] = ["content-type", "accept", "content-length"];

/// Why `label` can't name a new standalone, or `None`. `taken` holds labels already in use: the
/// zone's records, the platform's domains in it, and every instance's. The answer names the
/// rule, never another instance's label.
#[must_use]
pub fn check_label<'a>(label: &str, taken: impl IntoIterator<Item = &'a str>) -> Option<String> {
    if RESERVED.contains(&label) {
        return Some("that label is reserved".into());
    }
    let b = label.as_bytes();
    if !(3..=32).contains(&b.len()) {
        return Some("a label is 3 to 32 characters".into());
    }
    if !b
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
    {
        return Some("a label is lowercase letters, digits and `-`".into());
    }
    if !b[0].is_ascii_lowercase() {
        return Some("a label starts with a letter".into());
    }
    if b[b.len() - 1] == b'-' || label.contains("--") {
        return Some("a label doesn't end with `-` or hold `--`".into());
    }
    if taken.into_iter().any(|t| t == label) {
        return Some("that label is taken".into());
    }
    None
}

/// The host a label names under `zone`.
#[must_use]
pub fn host_of(label: &str, zone: &str) -> String {
    format!("{label}.{zone}")
}

/// The label of `host` when it sits directly under `zone` and could name a standalone, matched
/// without regard to case and a trailing dot; `None` otherwise.
#[must_use]
pub fn label_of(host: &str, zone: &str) -> Option<String> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let label = host.strip_suffix(&format!(".{}", zone.to_ascii_lowercase()))?;
    (!label.is_empty() && !label.contains('.') && check_label(label, []).is_none())
        .then(|| label.to_string())
}

/// Whether the proxy forwards a request for `path` at all.
#[must_use]
pub fn forwarded(path: &str) -> bool {
    path.starts_with(PATH_PREFIX) && !path.split('/').any(|s| s == "..")
}

/// Whether the proxy passes request header `name` to the standalone.
#[must_use]
pub fn passes(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    PASSED.contains(&n.as_str()) || n.starts_with("x-pigeon-")
}

#[cfg(test)]
mod tests {
    use super::{RESERVED, check_label, forwarded, host_of, label_of, passes};

    #[test]
    fn labels_are_dns_safe_unreserved_and_free() {
        assert_eq!(check_label("example", []), None);
        assert_eq!(check_label("ex-ample2", []), None);
        for (bad, why) in [
            ("ab", "3 to 32"),
            (&"a".repeat(33), "3 to 32"),
            ("Example", "lowercase"),
            ("ex_ample", "lowercase"),
            ("ex.ample", "lowercase"),
            ("1example", "starts with a letter"),
            ("-example", "starts with a letter"),
            ("example-", "`--`"),
            ("ex--ample", "`--`"),
            ("xn--example", "`--`"),
        ] {
            let got = check_label(bad, []).unwrap_or_default();
            assert!(got.contains(why), "{bad}: {got}");
        }
        for r in RESERVED {
            assert_eq!(
                check_label(r, []).as_deref(),
                Some("that label is reserved")
            );
        }
        assert_eq!(
            check_label("example", ["other", "example"]).as_deref(),
            Some("that label is taken")
        );
    }

    #[test]
    fn a_host_directly_under_the_zone_names_a_label() {
        let zone = "pigeon.dev";
        assert_eq!(host_of("example", zone), "example.pigeon.dev");
        assert_eq!(
            label_of("example.pigeon.dev", zone).as_deref(),
            Some("example")
        );
        assert_eq!(
            label_of("Example.Pigeon.dev.", zone).as_deref(),
            Some("example")
        );
        for not in [
            "pigeon.dev",
            "platform.pigeon.dev",
            "blue.platform.pigeon.dev",
            "a.example.pigeon.dev",
            "example.pigeon.dev.evil.dev",
            "examplepigeon.dev",
            "example.workers.dev",
        ] {
            assert_eq!(label_of(not, zone), None, "{not}");
        }
    }

    #[test]
    fn the_proxy_forwards_only_app_paths_and_safe_headers() {
        assert!(forwarded("/v0/app/instances"));
        for p in [
            "/",
            "/healthz",
            "/builds/x",
            "/v0/apps",
            "/v0/app/../mcp",
            "/mcp",
        ] {
            assert!(!forwarded(p), "{p}");
        }
        for h in [
            "Content-Type",
            "accept",
            "X-Pigeon-Device",
            "x-pigeon-proof",
        ] {
            assert!(passes(h), "{h}");
        }
        for h in [
            "Cookie",
            "Authorization",
            "Cf-Access-Jwt-Assertion",
            "cf-connecting-ip",
            "X-Forwarded-For",
            "Host",
        ] {
            assert!(!passes(h), "{h}");
        }
    }
}
