//! Claude Platform's Messages API as a gateway; see
//! `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`.

use serde_json::Value;

use super::{
    Micros,
    config::Pool,
    cost,
    gateway::{Admitted, Gateway, Refused, Usage},
    ledger::Signal,
};

/// Claude Platform's Messages API, `anthropic-messages`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Messages;

/// The messages path.
pub const MESSAGES: &str = "/v1/messages";
/// The token counting path.
pub const COUNT: &str = "/v1/messages/count_tokens";
/// The most `max_tokens` a request may ask for.
pub const MAX_TOKENS: u64 = 64_000;
/// Tokens added to the body's size bound for what the API adds.
pub const PREAMBLE: u64 = 4_096;
/// Request keys that are refused.
pub const REFUSED_KEYS: [&str; 4] = ["speed", "fallbacks", "mcp_servers", "container"];
/// The price of one server tool use, in micro-dollars.
pub const SERVER_TOOL: Micros = 10_000;

fn refuse(text: &str) -> Refused {
    Refused(format!("pool: {text}"))
}

/// A JSON value's text, a string without its quotes.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Set `dst` from `u[key]`; a missing number is 0 when `all`, else `dst` stays.
fn set(dst: &mut u64, u: &Value, key: &str, all: bool) {
    match u.get(key).and_then(Value::as_u64) {
        Some(n) => *dst = n,
        None if all => *dst = 0,
        None => {}
    }
}

/// Apply the usage object `u` to `acc`; with `all`, a field it lacks is reset.
fn apply(acc: &mut Usage, u: &Value, all: bool) {
    set(&mut acc.input, u, "input_tokens", all);
    set(&mut acc.read, u, "cache_read_input_tokens", all);
    set(&mut acc.output, u, "output_tokens", all);
    let total = u.get("cache_creation_input_tokens").and_then(Value::as_u64);
    if let Some(cc) = u.get("cache_creation").filter(|c| c.is_object()) {
        set(&mut acc.write_5m, cc, "ephemeral_5m_input_tokens", true);
        set(&mut acc.write_1h, cc, "ephemeral_1h_input_tokens", true);
        acc.write_unsplit = total
            .unwrap_or(0)
            .saturating_sub(acc.write_5m)
            .saturating_sub(acc.write_1h);
    } else if let Some(t) = total {
        acc.write_unsplit = t;
    } else if all {
        acc.write_unsplit = 0;
    }
    if all || u.get("inference_geo").is_some() {
        acc.us_only = u.get("inference_geo").and_then(Value::as_str) == Some("us");
    }
    if let Some(tools) = u.get("server_tool_use").and_then(Value::as_object) {
        acc.server_tools = tools
            .values()
            .filter_map(Value::as_u64)
            .fold(0, u64::saturating_add);
    } else if all {
        acc.server_tools = 0;
    }
}

impl Gateway for Messages {
    fn name(&self) -> &'static str {
        "anthropic-messages"
    }

    fn admit(&self, path: &str, body: &[u8], models: &[String]) -> Result<Admitted, Refused> {
        if path != MESSAGES && path != COUNT {
            return Err(refuse("only /v1/messages and /v1/messages/count_tokens"));
        }
        let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let Some(obj) = value.as_object() else {
            return Err(refuse("the body is not a JSON object"));
        };
        let model = obj.get("model").cloned().unwrap_or(Value::Null);
        let allowed = model
            .as_str()
            .filter(|m| models.iter().any(|a| a == m))
            .map(str::to_owned);
        let Some(model) = allowed else {
            return Err(refuse(&format!("model `{}` is not allowed", text(&model))));
        };
        if let Some(key) = REFUSED_KEYS.iter().find(|k| obj.contains_key(**k)) {
            return Err(refuse(&format!("`{key}` is refused")));
        }
        if obj
            .get("inference_geo")
            .is_some_and(|g| g.as_str() != Some("global"))
        {
            return Err(refuse("`inference_geo` other than global is refused"));
        }
        if let Some(tools) = obj.get("tools") {
            let items: &[Value] = tools.as_array().map_or(std::slice::from_ref(tools), |a| a);
            for tool in items {
                let bad = match tool.as_object() {
                    None => Some(tool),
                    Some(t) => t.get("type").filter(|ty| ty.as_str() != Some("custom")),
                };
                if let Some(ty) = bad {
                    return Err(refuse(&format!("tool type `{}` is refused", text(ty))));
                }
            }
        }
        let max_tokens = if path == MESSAGES {
            let Some(n) = obj.get("max_tokens").and_then(Value::as_u64) else {
                return Err(refuse("`max_tokens` is missing"));
            };
            if n > MAX_TOKENS {
                return Err(refuse(&format!(
                    "`max_tokens` above {MAX_TOKENS} is refused"
                )));
            }
            n
        } else {
            0
        };
        Ok(Admitted {
            path: path.to_owned(),
            model,
            body_bytes: body.len() as u64,
            max_tokens,
            stream: obj.get("stream") == Some(&Value::Bool(true)),
        })
    }

    fn reserve(&self, a: &Admitted, pool: &Pool) -> Micros {
        if a.path == COUNT {
            return 0;
        }
        let bound = a.body_bytes.saturating_add(PREAMBLE);
        let Some(p) = pool.price(&a.model, bound).or_else(|| pool.priciest()) else {
            return u64::MAX;
        };
        cost(bound, p.write_1h).saturating_add(cost(a.max_tokens, p.output))
    }

    fn usage_json(&self, body: &[u8]) -> Option<Usage> {
        let v: Value = serde_json::from_slice(body).ok()?;
        let u = v.as_object()?.get("usage").filter(|u| u.is_object())?;
        let mut out = Usage {
            model: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            ..Usage::default()
        };
        apply(&mut out, u, true);
        out.complete = true;
        Some(out)
    }

    fn usage_event(&self, acc: &Usage, data: &str) -> Usage {
        let mut out = acc.clone();
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return out;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(m) = v.get("message") {
                    m.get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .clone_into(&mut out.model);
                    if let Some(u) = m.get("usage").filter(|u| u.is_object()) {
                        apply(&mut out, u, true);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                    apply(&mut out, u, false);
                }
            }
            Some("message_stop") => out.complete = true,
            _ => {}
        }
        out
    }

    fn cost(&self, u: &Usage, pool: &Pool) -> Micros {
        let prompt = u
            .input
            .saturating_add(u.write_5m)
            .saturating_add(u.write_1h)
            .saturating_add(u.write_unsplit)
            .saturating_add(u.read);
        let Some(p) = pool.price(&u.model, prompt).or_else(|| pool.priciest()) else {
            return u64::MAX;
        };
        let mut sum = cost(u.input, p.input)
            .saturating_add(cost(u.write_5m, p.write_5m))
            .saturating_add(cost(u.write_1h.saturating_add(u.write_unsplit), p.write_1h))
            .saturating_add(cost(u.read, p.read))
            .saturating_add(cost(u.output, p.output));
        if u.us_only {
            sum = sum.saturating_add(sum.div_ceil(10));
        }
        sum.saturating_add(u.server_tools.saturating_mul(SERVER_TOOL))
    }

    fn fresh_tokens(&self, u: &Usage) -> u64 {
        u.input
            .saturating_add(u.write_5m)
            .saturating_add(u.write_1h)
            .saturating_add(u.write_unsplit)
            .saturating_add(u.output)
    }

    fn classify(&self, status: u16, body: &str) -> Signal {
        if (200..=299).contains(&status) {
            Signal::Ok
        } else if body.contains("credit balance is too low") {
            Signal::Exhausted
        } else if body.contains("specified workspace API usage limits")
            || body.contains("specified API usage limits")
            || body.contains("enforced_spend_limit_reached")
        {
            Signal::Limited
        } else if status == 401 || status == 403 {
            Signal::Unauthorized
        } else if status == 429 {
            Signal::RateLimited
        } else {
            Signal::Ok
        }
    }

    fn key_header(&self) -> &'static str {
        "x-api-key"
    }

    fn forwarded_headers(&self) -> &'static [&'static str] {
        &["anthropic-version", "anthropic-beta"]
    }

    fn account_header(&self) -> Option<&'static str> {
        Some("anthropic-workspace-id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::config::config;

    fn pool() -> Pool {
        config().expect("config").pools["anthropic"].clone()
    }

    fn models() -> Vec<String> {
        vec!["claude-sonnet-5-5".into(), "claude-haiku-5-5".into()]
    }

    fn admit(path: &str, body: &str) -> Result<Admitted, Refused> {
        Messages.admit(path, body.as_bytes(), &models())
    }

    fn refused(body: &str) -> String {
        admit(MESSAGES, body).expect_err("refused").0
    }

    const OK: &str =
        r#"{"model":"claude-sonnet-5-5","max_tokens":1000,"stream":true,"messages":[]}"#;
    const USAGE: &str = r#"{"model":"claude-sonnet-5-5","usage":{"input_tokens":12,"cache_creation_input_tokens":3000,"cache_creation":{"ephemeral_5m_input_tokens":1000,"ephemeral_1h_input_tokens":2000},"cache_read_input_tokens":50000,"output_tokens":800}}"#;

    fn usage() -> Usage {
        Messages.usage_json(USAGE.as_bytes()).expect("usage")
    }

    #[test]
    fn names_and_headers() {
        assert_eq!(Messages.name(), "anthropic-messages");
        assert_eq!(Messages.key_header(), "x-api-key");
        assert_eq!(
            Messages.forwarded_headers(),
            ["anthropic-version", "anthropic-beta"]
        );
        assert_eq!(Messages.account_header(), Some("anthropic-workspace-id"));
    }

    #[test]
    fn admits_a_minimal_streaming_request() {
        let a = admit(MESSAGES, OK).expect("admitted");
        assert_eq!(a.path, MESSAGES);
        assert_eq!(a.model, "claude-sonnet-5-5");
        assert_eq!(a.body_bytes, OK.len() as u64);
        assert_eq!(a.max_tokens, 1000);
        assert!(a.stream);
    }

    #[test]
    fn stream_defaults_to_false() {
        let a = admit(MESSAGES, r#"{"model":"claude-haiku-5-5","max_tokens":5}"#).expect("ok");
        assert!(!a.stream);
    }

    #[test]
    fn refuses_other_paths() {
        let r = admit("/v1/models", OK).expect_err("refused");
        assert_eq!(r.0, "pool: only /v1/messages and /v1/messages/count_tokens");
    }

    #[test]
    fn refuses_a_body_that_is_not_an_object() {
        for body in ["[]", "7", "not json", ""] {
            assert_eq!(
                refused(body),
                "pool: the body is not a JSON object",
                "{body}"
            );
        }
    }

    #[test]
    fn refuses_a_model_not_listed() {
        assert_eq!(
            refused(r#"{"model":"gpt","max_tokens":1}"#),
            "pool: model `gpt` is not allowed"
        );
    }

    #[test]
    fn refuses_a_model_that_is_not_a_string() {
        assert_eq!(
            refused(r#"{"model":7,"max_tokens":1}"#),
            "pool: model `7` is not allowed"
        );
    }

    #[test]
    fn refuses_each_refused_key() {
        for key in REFUSED_KEYS {
            let body = format!(r#"{{"model":"claude-sonnet-5-5","max_tokens":1,"{key}":1}}"#);
            assert_eq!(refused(&body), format!("pool: `{key}` is refused"));
        }
    }

    #[test]
    fn refuses_inference_geo_other_than_global() {
        for geo in [r#""us""#, "1"] {
            let body =
                format!(r#"{{"model":"claude-sonnet-5-5","max_tokens":1,"inference_geo":{geo}}}"#);
            assert_eq!(
                refused(&body),
                "pool: `inference_geo` other than global is refused"
            );
        }
    }

    #[test]
    fn allows_inference_geo_global() {
        let body = r#"{"model":"claude-sonnet-5-5","max_tokens":1,"inference_geo":"global"}"#;
        assert!(admit(MESSAGES, body).is_ok());
    }

    #[test]
    fn refuses_a_server_tool() {
        let body = r#"{"model":"claude-sonnet-5-5","max_tokens":1,"tools":[{"type":"web_search_20250305","name":"w"}]}"#;
        assert_eq!(
            refused(body),
            "pool: tool type `web_search_20250305` is refused"
        );
    }

    #[test]
    fn allows_custom_tools() {
        let body = r#"{"model":"claude-sonnet-5-5","max_tokens":1,"tools":[{"name":"a"},{"type":"custom","name":"b"}]}"#;
        assert!(admit(MESSAGES, body).is_ok());
    }

    #[test]
    fn refuses_a_tool_that_is_not_an_object() {
        let body = r#"{"model":"claude-sonnet-5-5","max_tokens":1,"tools":["x"]}"#;
        assert_eq!(refused(body), "pool: tool type `x` is refused");
    }

    #[test]
    fn refuses_a_missing_max_tokens() {
        for body in [
            r#"{"model":"claude-sonnet-5-5"}"#,
            r#"{"model":"claude-sonnet-5-5","max_tokens":"9"}"#,
            r#"{"model":"claude-sonnet-5-5","max_tokens":-1}"#,
        ] {
            assert_eq!(refused(body), "pool: `max_tokens` is missing", "{body}");
        }
    }

    #[test]
    fn refuses_max_tokens_above_the_limit() {
        let body = r#"{"model":"claude-sonnet-5-5","max_tokens":64001}"#;
        assert_eq!(refused(body), "pool: `max_tokens` above 64000 is refused");
        let body = r#"{"model":"claude-sonnet-5-5","max_tokens":64000}"#;
        assert!(admit(MESSAGES, body).is_ok());
    }

    #[test]
    fn the_first_failing_check_wins() {
        let body = r#"{"model":"gpt","speed":"fast"}"#;
        assert_eq!(refused(body), "pool: model `gpt` is not allowed");
        let body = r#"{"model":"claude-sonnet-5-5","speed":"fast","inference_geo":"us"}"#;
        assert_eq!(refused(body), "pool: `speed` is refused");
    }

    #[test]
    fn count_is_admitted_with_no_max_tokens() {
        let a = admit(COUNT, r#"{"model":"claude-sonnet-5-5","messages":[]}"#).expect("ok");
        assert_eq!(a.max_tokens, 0);
        assert_eq!(a.path, COUNT);
    }

    fn admitted(model: &str, body_bytes: u64, max_tokens: u64, path: &str) -> Admitted {
        Admitted {
            path: path.into(),
            model: model.into(),
            body_bytes,
            max_tokens,
            stream: false,
        }
    }

    #[test]
    fn reserve_prices_the_body_and_max_tokens() {
        let a = admitted("claude-sonnet-5-5", 10_000, 32_000, MESSAGES);
        assert_eq!(
            Messages.reserve(&a, &pool()),
            cost(14_096, 4_000_000) + cost(32_000, 10_000_000)
        );
    }

    #[test]
    fn reserve_uses_the_long_price_for_a_big_body() {
        let a = admitted("claude-haiku-5-5", 200_000, 1_000, MESSAGES);
        assert_eq!(
            Messages.reserve(&a, &pool()),
            cost(204_096, 1_000_000) + cost(1_000, 2_500_000)
        );
    }

    #[test]
    fn reserve_of_a_count_is_free() {
        let a = admitted("claude-sonnet-5-5", 10_000, 0, COUNT);
        assert_eq!(Messages.reserve(&a, &pool()), 0);
    }

    #[test]
    fn reserve_of_an_unknown_model_uses_the_priciest() {
        let a = admitted("claude-other", 10_000, 1_000, MESSAGES);
        let p = pool().priciest().expect("priciest");
        assert_eq!(
            Messages.reserve(&a, &pool()),
            cost(14_096, p.write_1h) + cost(1_000, p.output)
        );
    }

    #[test]
    fn reserve_with_no_prices_is_max() {
        let mut p = pool();
        p.prices.clear();
        p.long.clear();
        let a = admitted("claude-other", 10, 10, MESSAGES);
        assert_eq!(Messages.reserve(&a, &p), u64::MAX);
    }

    #[test]
    fn usage_json_reads_a_split_cache_write() {
        let u = usage();
        assert_eq!(u.model, "claude-sonnet-5-5");
        assert_eq!(u.input, 12);
        assert_eq!(u.write_5m, 1000);
        assert_eq!(u.write_1h, 2000);
        assert_eq!(u.write_unsplit, 0);
        assert_eq!(u.read, 50_000);
        assert_eq!(u.output, 800);
        assert!(u.complete);
        assert!(!u.us_only);
        assert_eq!(u.server_tools, 0);
    }

    #[test]
    fn usage_json_without_a_split_keeps_the_write_unsplit() {
        let body =
            r#"{"usage":{"input_tokens":1,"cache_creation_input_tokens":3000,"output_tokens":2}}"#;
        let u = Messages.usage_json(body.as_bytes()).expect("usage");
        assert_eq!(u.write_unsplit, 3000);
        assert_eq!((u.write_5m, u.write_1h), (0, 0));
        assert_eq!(u.model, "");
    }

    #[test]
    fn usage_json_leaves_the_unsplit_remainder() {
        let body = r#"{"usage":{"cache_creation_input_tokens":3500,"cache_creation":{"ephemeral_5m_input_tokens":1000,"ephemeral_1h_input_tokens":2000}}}"#;
        let u = Messages.usage_json(body.as_bytes()).expect("usage");
        assert_eq!(u.write_unsplit, 500);
    }

    #[test]
    fn usage_json_reads_geo_and_server_tools() {
        let body = r#"{"usage":{"inference_geo":"us","server_tool_use":{"web_search_requests":2,"web_fetch_requests":1}}}"#;
        let u = Messages.usage_json(body.as_bytes()).expect("usage");
        assert!(u.us_only);
        assert_eq!(u.server_tools, 3);
    }

    #[test]
    fn usage_json_is_none_without_usage() {
        for body in [r#"{"model":"m"}"#, r#"{"usage":3}"#, "[]", "nope"] {
            assert!(Messages.usage_json(body.as_bytes()).is_none(), "{body}");
        }
    }

    fn event(acc: &Usage, data: &str) -> Usage {
        Messages.usage_event(acc, data)
    }

    #[test]
    fn usage_events_accumulate_a_stream() {
        let mut u = Usage::default();
        u = event(
            &u,
            r#"{"type":"message_start","message":{"model":"claude-sonnet-5-5","usage":{"input_tokens":12,"output_tokens":1}}}"#,
        );
        assert_eq!((u.input, u.output, u.complete), (12, 1, false));
        assert_eq!(u.model, "claude-sonnet-5-5");
        u = event(&u, r#"{"type":"content_block_delta","delta":{}}"#);
        assert_eq!(u.output, 1);
        u = event(
            &u,
            r#"{"type":"message_delta","usage":{"output_tokens":640}}"#,
        );
        assert_eq!((u.input, u.output), (12, 640));
        u = event(&u, r#"{"type":"message_stop"}"#);
        assert_eq!((u.input, u.output, u.complete), (12, 640, true));
    }

    #[test]
    fn a_delta_keeps_the_fields_it_lacks() {
        let start = event(
            &Usage::default(),
            r#"{"type":"message_start","message":{"model":"m","usage":{"input_tokens":5,"cache_read_input_tokens":9}}}"#,
        );
        let u = event(
            &start,
            r#"{"type":"message_delta","usage":{"input_tokens":6}}"#,
        );
        assert_eq!((u.input, u.read), (6, 9));
    }

    #[test]
    fn a_non_json_event_is_ignored() {
        let acc = usage();
        assert_eq!(event(&acc, "not json"), acc);
        assert_eq!(event(&acc, r#"{"type":"ping"}"#), acc);
    }

    #[test]
    fn cost_sums_each_kind_at_its_price() {
        let want = cost(12, 2_000_000)
            + cost(1000, 2_500_000)
            + cost(2000, 4_000_000)
            + cost(50_000, 100_000)
            + cost(800, 10_000_000);
        assert_eq!(Messages.cost(&usage(), &pool()), want);
    }

    #[test]
    fn cost_adds_a_tenth_for_us_only() {
        let mut u = usage();
        let base = Messages.cost(&u, &pool());
        u.us_only = true;
        assert_eq!(Messages.cost(&u, &pool()), base + base.div_ceil(10));
    }

    #[test]
    fn cost_adds_server_tools() {
        let mut u = usage();
        let base = Messages.cost(&u, &pool());
        u.server_tools = 3;
        assert_eq!(Messages.cost(&u, &pool()), base + 30_000);
    }

    #[test]
    fn cost_uses_the_long_price_for_a_long_prompt() {
        let u = Usage {
            model: "claude-haiku-5-5".into(),
            input: 150_000,
            output: 1_000,
            ..Usage::default()
        };
        assert_eq!(
            Messages.cost(&u, &pool()),
            cost(150_000, 500_000) + cost(1_000, 2_500_000)
        );
    }

    #[test]
    fn cost_of_an_unknown_model_uses_the_priciest_or_max() {
        let u = Usage {
            model: "other".into(),
            input: 1_000_000,
            ..Usage::default()
        };
        let p = pool().priciest().expect("priciest");
        assert_eq!(Messages.cost(&u, &pool()), p.input);
        let mut none = pool();
        none.prices.clear();
        none.long.clear();
        assert_eq!(Messages.cost(&u, &none), u64::MAX);
    }

    #[test]
    fn cost_saturates() {
        let u = Usage {
            model: "claude-sonnet-5-5".into(),
            input: u64::MAX,
            server_tools: u64::MAX,
            us_only: true,
            ..Usage::default()
        };
        assert_eq!(Messages.cost(&u, &pool()), u64::MAX);
    }

    #[test]
    fn fresh_tokens_leave_out_reads() {
        assert_eq!(Messages.fresh_tokens(&usage()), 3812);
    }

    #[test]
    fn classifies_responses() {
        let c = |s, b| Messages.classify(s, b);
        assert_eq!(c(200, ""), Signal::Ok);
        assert_eq!(
            c(
                400,
                "Your credit balance is too low to access the Anthropic API."
            ),
            Signal::Exhausted
        );
        assert_eq!(
            c(
                400,
                "You have reached your specified workspace API usage limits"
            ),
            Signal::Limited
        );
        assert_eq!(
            c(400, "You have reached your specified API usage limits"),
            Signal::Limited
        );
        assert_eq!(c(429, "enforced_spend_limit_reached"), Signal::Limited);
        assert_eq!(c(401, ""), Signal::Unauthorized);
        assert_eq!(c(403, ""), Signal::Unauthorized);
        assert_eq!(c(429, "rate limit"), Signal::RateLimited);
        assert_eq!(c(500, "oops"), Signal::Ok);
    }

    #[test]
    fn a_success_is_ok_whatever_the_body_says() {
        assert_eq!(
            Messages.classify(200, "credit balance is too low"),
            Signal::Ok
        );
    }
}
