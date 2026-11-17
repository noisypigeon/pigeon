//! The AI Gateway upstream for a pool's Messages requests, pure; see
//! `.claude/decisions/2026-11-16-security-ai-gateway-claude-api.md`.

use regex::Regex;

/// The upstream request path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// A Messages request.
    Messages,
    /// A token count request.
    CountTokens,
}

impl Path {
    /// The path's suffix under the gateway.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Path::Messages => "/anthropic/v1/messages",
            Path::CountTokens => "/anthropic/v1/messages/count_tokens",
        }
    }
}

fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern).is_ok_and(|r| r.is_match(text))
}

/// The gateway URL for `path`, or None when a part is malformed.
#[must_use]
pub fn url(base: &str, account: &str, gateway: &str, path: Path) -> Option<String> {
    if !base.starts_with("https://")
        || base.ends_with('/')
        || !matches("^[0-9a-f]{32}$", account)
        || !matches("^[a-z0-9][a-z0-9-]{0,63}$", gateway)
    {
        return None;
    }
    Some(format!("{base}/v1/{account}/{gateway}{}", path.suffix()))
}

/// The fields a request's `cf-aig-metadata` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// The pool's name.
    pub pool: String,
    /// The task's id.
    pub task: String,
    /// The change's number.
    pub change: u64,
    /// The brief's id.
    pub brief: String,
    /// The model class.
    pub class: String,
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("a string serializes")
}

/// The `cf-aig-metadata` value for `meta`.
///
/// # Errors
/// The first bad field's message.
pub fn metadata(meta: &Meta) -> Result<String, String> {
    if !matches("^[a-z][a-z0-9-]{0,31}$", &meta.pool) {
        return Err("metadata: bad pool".to_string());
    }
    if !matches("^[A-Za-z0-9._:-]{1,64}$", &meta.task) {
        return Err("metadata: bad task".to_string());
    }
    if meta.change == 0 {
        return Err("metadata: bad change".to_string());
    }
    if !matches("^b[0-9]{1,6}$", &meta.brief) {
        return Err("metadata: bad brief".to_string());
    }
    if meta.class != "haiku" && meta.class != "sonnet" {
        return Err("metadata: bad class".to_string());
    }
    Ok(format!(
        "{{\"pool\":{},\"task\":{},\"change\":{},\"brief\":{},\"class\":{}}}",
        quoted(&meta.pool),
        quoted(&meta.task),
        meta.change,
        quoted(&meta.brief),
        quoted(&meta.class)
    ))
}

/// The upstream request timeout, in milliseconds.
pub const REQUEST_TIMEOUT_MS: u64 = 120_000;
/// The caller headers sent upstream.
pub const KEPT: [&str; 2] = ["anthropic-version", "anthropic-beta"];

/// The headers the Worker sends upstream, names lowercase.
///
/// # Errors
/// `metadata`'s message, unchanged.
pub fn outbound(
    caller: &[(String, String)],
    meta: &Meta,
    token: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = caller
        .iter()
        .filter(|(n, _)| KEPT.contains(&n.to_ascii_lowercase().as_str()))
        .map(|(n, v)| (n.to_ascii_lowercase(), v.clone()))
        .collect();
    let mut push = |n: &str, v: String| out.push((n.to_string(), v));
    push("content-type", "application/json".to_string());
    if let Some(t) = token {
        push("cf-aig-authorization", format!("Bearer {t}"));
    }
    push("cf-aig-metadata", metadata(meta)?);
    push("cf-aig-skip-cache", "true".to_string());
    push("cf-aig-collect-log-payload", "false".to_string());
    push("cf-aig-request-timeout", REQUEST_TIMEOUT_MS.to_string());
    Ok(out)
}

/// What a failed upstream response tells the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Anthropic's own error.
    Upstream,
    /// The gateway's rate limit.
    GatewayLimited,
    /// The gateway has no stored key.
    GatewayNoKey,
    /// The gateway refused the token.
    GatewayUnauthorized,
    /// Any other gateway error, by code.
    Gateway(i64),
}

impl Signal {
    /// Whether the signal pauses the pool.
    #[must_use]
    pub fn pauses(self) -> bool {
        matches!(
            self,
            Signal::GatewayNoKey | Signal::GatewayUnauthorized | Signal::Gateway(_)
        )
    }
}

/// The signal for a response, or None for a success.
#[must_use]
pub fn classify(status: u16, body: &str) -> Option<Signal> {
    if (200..300).contains(&status) {
        return None;
    }
    let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
    if json
        .as_ref()
        .and_then(|v| v.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("error")
    {
        return Some(Signal::Upstream);
    }
    let code = json.as_ref().and_then(|v| {
        let list = v
            .get("error")
            .and_then(|e| e.as_array())
            .or_else(|| v.get("errors").and_then(|e| e.as_array()))?;
        list.first()?.get("code")?.as_i64()
    });
    Some(match (code, status) {
        (Some(2044), _) => Signal::GatewayNoKey,
        (_, 401) => Signal::GatewayUnauthorized,
        (_, 429) => Signal::GatewayLimited,
        (Some(c), _) => Signal::Gateway(c),
        (None, _) => Signal::Gateway(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
    const GATEWAY: &str = "pigeon-claude";
    const BASE: &str = "https://gw.example";

    fn meta() -> Meta {
        Meta {
            pool: "anthropic".into(),
            task: "t1".into(),
            change: 813,
            brief: "b60".into(),
            class: "sonnet".into(),
        }
    }

    #[test]
    fn url_builds_both_paths() {
        assert_eq!(
            url(BASE, ACCOUNT, GATEWAY, Path::Messages).as_deref(),
            Some(
                "https://gw.example/v1/0123456789abcdef0123456789abcdef/pigeon-claude/anthropic/v1/messages"
            )
        );
        assert_eq!(
            url(BASE, ACCOUNT, GATEWAY, Path::CountTokens).as_deref(),
            Some(
                "https://gw.example/v1/0123456789abcdef0123456789abcdef/pigeon-claude/anthropic/v1/messages/count_tokens"
            )
        );
    }

    #[test]
    fn url_refuses_bad_parts() {
        let p = Path::Messages;
        assert_eq!(url("http://gw.example", ACCOUNT, GATEWAY, p), None);
        assert_eq!(url("https://gw.example/", ACCOUNT, GATEWAY, p), None);
        assert_eq!(url(BASE, &ACCOUNT[..31], GATEWAY, p), None);
        assert_eq!(
            url(BASE, "0123456789abcdef0123456789abcdeF", GATEWAY, p),
            None
        );
        assert_eq!(url(BASE, ACCOUNT, "Pigeon", p), None);
        assert_eq!(url(BASE, ACCOUNT, "-x", p), None);
        assert_eq!(url(BASE, ACCOUNT, &"a".repeat(65), p), None);
    }

    #[test]
    fn metadata_is_ordered_compact_json() {
        assert_eq!(
            metadata(&meta()).as_deref(),
            Ok(
                "{\"pool\":\"anthropic\",\"task\":\"t1\",\"change\":813,\"brief\":\"b60\",\"class\":\"sonnet\"}"
            )
        );
    }

    #[test]
    fn metadata_refuses_in_order() {
        let bad = Meta {
            pool: "Bad".into(),
            task: String::new(),
            change: 0,
            brief: "x".into(),
            class: "opus".into(),
        };
        assert_eq!(metadata(&bad), Err("metadata: bad pool".to_string()));
        let m = |f: fn(&mut Meta)| {
            let mut x = meta();
            f(&mut x);
            metadata(&x)
        };
        assert_eq!(
            m(|x| x.pool = "Bad".into()),
            Err("metadata: bad pool".into())
        );
        assert_eq!(
            m(|x| x.task = String::new()),
            Err("metadata: bad task".into())
        );
        assert_eq!(m(|x| x.change = 0), Err("metadata: bad change".into()));
        assert_eq!(
            m(|x| x.brief = "x".into()),
            Err("metadata: bad brief".into())
        );
        assert_eq!(
            m(|x| x.class = "opus".into()),
            Err("metadata: bad class".into())
        );
    }

    fn h(n: &str, v: &str) -> (String, String) {
        (n.to_string(), v.to_string())
    }

    #[test]
    fn outbound_keeps_only_versions_and_adds_gateway_headers() {
        let caller = vec![
            h("X-Api-Key", "k"),
            h("Anthropic-Version", "2023-06-01"),
            h("cf-aig-metadata", "{}"),
            h("Authorization", "Bearer x"),
            h("anthropic-beta", "b1"),
            h("cf-aig-byok-alias", "other"),
        ];
        let got = outbound(&caller, &meta(), Some("tok")).unwrap();
        let want = vec![
            h("anthropic-version", "2023-06-01"),
            h("anthropic-beta", "b1"),
            h("content-type", "application/json"),
            h("cf-aig-authorization", "Bearer tok"),
            h("cf-aig-metadata", &metadata(&meta()).unwrap()),
            h("cf-aig-skip-cache", "true"),
            h("cf-aig-collect-log-payload", "false"),
            h("cf-aig-request-timeout", "120000"),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn outbound_without_token_has_no_authorization() {
        let caller = vec![h("x-api-key", "k")];
        let got = outbound(&caller, &meta(), None).unwrap();
        assert!(
            got.iter()
                .all(|(n, _)| n != "cf-aig-authorization" && n != "x-api-key")
        );
    }

    #[test]
    fn outbound_returns_metadata_errors() {
        let mut m = meta();
        m.class = "opus".into();
        assert_eq!(
            outbound(&[], &m, None),
            Err("metadata: bad class".to_string())
        );
    }

    #[test]
    fn classify_success_is_none() {
        assert_eq!(classify(200, "{}"), None);
        assert_eq!(classify(204, ""), None);
    }

    #[test]
    fn classify_anthropic_errors_are_upstream() {
        let a = r#"{"type":"error","error":{"type":"rate_limit_error","message":"m"}}"#;
        let b = r#"{"type":"error","error":{"type":"invalid_request_error","message":"credit balance is too low"}}"#;
        assert_eq!(classify(429, a), Some(Signal::Upstream));
        assert_eq!(classify(400, b), Some(Signal::Upstream));
    }

    #[test]
    fn classify_gateway_codes() {
        let a = r#"{"success":false,"error":[{"code":2044,"message":"m"}]}"#;
        let b = r#"{"success":false,"errors":[{"code":10000,"message":"m"}]}"#;
        let c = r#"{"success":false,"error":[{"code":2003,"message":"m"}]}"#;
        let d = r#"{"error":[{"code":2005}]}"#;
        assert_eq!(classify(400, a), Some(Signal::GatewayNoKey));
        assert_eq!(classify(401, b), Some(Signal::GatewayUnauthorized));
        assert_eq!(classify(429, c), Some(Signal::GatewayLimited));
        assert_eq!(classify(500, d), Some(Signal::Gateway(2005)));
    }

    #[test]
    fn classify_non_json() {
        assert_eq!(classify(401, "nope"), Some(Signal::GatewayUnauthorized));
        assert_eq!(classify(429, "nope"), Some(Signal::GatewayLimited));
        assert_eq!(classify(502, "<html>"), Some(Signal::Gateway(0)));
    }

    #[test]
    fn pauses_matches_the_adr() {
        assert!(!Signal::Upstream.pauses());
        assert!(!Signal::GatewayLimited.pauses());
        assert!(Signal::GatewayNoKey.pauses());
        assert!(Signal::GatewayUnauthorized.pauses());
        assert!(Signal::Gateway(7).pauses());
    }
}
