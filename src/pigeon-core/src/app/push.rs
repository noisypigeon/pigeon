//! The app's push notifications through APNs: the provider token's signing input, the payload,
//! and what an answer says; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §6.

use serde_json::{Value, json};

use crate::push::b64url;

/// APNs' production host, which `TestFlight` and App Store builds use.
pub const HOST: &str = "https://api.push.apple.com";
/// The app's bundle ID, APNs' topic.
pub const TOPIC: &str = "dev.pigeon.app";
/// How long a provider token is reused, in seconds; APNs refuses one older than an hour.
pub const TOKEN_TTL: u64 = 50 * 60;

/// The provider token's signing input: ES256 with key `kid`, issued by team `team` at `now`.
#[must_use]
pub fn token_input(kid: &str, team: &str, now: u64) -> String {
    let header = json!({ "alg": "ES256", "kid": kid });
    let claims = json!({ "iss": team, "iat": now });
    format!(
        "{}.{}",
        b64url(header.to_string().as_bytes()),
        b64url(claims.to_string().as_bytes())
    )
}

/// How many open items need the owner, in words: `1 needs you`, else `<count> need you`.
#[must_use]
pub fn said(count: usize) -> String {
    if count == 1 {
        "1 needs you".to_string()
    } else {
        format!("{count} need you")
    }
}

/// The alert: a title, `count` in words and as the badge, and nothing else; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md` §5.
#[must_use]
pub fn payload(count: usize) -> Value {
    json!({
        "aps": {
            "alert": { "title": "pigeon", "body": said(count) },
            "badge": count,
            "sound": "default",
        },
    })
}

/// Whether APNs' answer `status` and `reason` mean the device token is gone for good.
#[must_use]
pub fn gone(status: u16, reason: &str) -> bool {
    status == 410 || (status == 400 && reason == "BadDeviceToken")
}

/// Whether `token` looks like an APNs device token: hex, 64 to 200 characters.
#[must_use]
pub fn token_ok(token: &str) -> bool {
    (64..=200).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
}
