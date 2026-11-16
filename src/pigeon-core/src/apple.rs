//! The Apple connection's functional core: App Store Connect tokens, and the checks that let a
//! Buildkite `sign` step redeem a release's signing identity; see
//! `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md` §5.

use serde_json::{Value, json};

use crate::control::Statement;
use crate::push::b64url;

/// App Store Connect's token audience.
pub const ASC_AUDIENCE: &str = "appstoreconnect-v1";
/// The longest App Store Connect accepts a token for, in seconds.
pub const ASC_MAX_TTL: u64 = 20 * 60;
/// App Store Connect's API.
pub const ASC_API: &str = "https://api.appstoreconnect.apple.com";
/// The app's bundle ID.
pub const BUNDLE_ID: &str = "dev.pigeon.app";
/// Buildkite's OIDC issuer.
pub const BUILDKITE_ISSUER: &str = "https://agent.buildkite.com";
/// Buildkite's OIDC keys.
pub const BUILDKITE_JWKS: &str = "https://agent.buildkite.com/.well-known/jwks";
/// The Buildkite organization the signing pipeline lives in.
pub const ORGANIZATION: &str = "willow-graysen";
/// The signing pipeline.
pub const PIPELINE: &str = "pigeon-ios-release";
/// The one step that may redeem a grant.
pub const STEP: &str = "sign";
/// The only branch released.
pub const BRANCH: &str = "main";
/// The route a `sign` step redeems its grant at; Access lets it through, so it authenticates
/// the caller itself.
pub const ROUTE: &str = "/v0/apple/release";
/// Leeway for clocks, in seconds.
const SKEW: u64 = 60;

/// The `signing_input` of an App Store Connect token for key `key_id` of team `issuer`, issued
/// `now` and valid for `ttl` seconds, at most [`ASC_MAX_TTL`].
#[must_use]
pub fn asc_jwt_input(key_id: &str, issuer: &str, now: u64, ttl: u64) -> String {
    let header = json!({ "alg": "ES256", "kid": key_id, "typ": "JWT" });
    let claims = json!({
        "iss": issuer,
        "iat": now,
        "exp": now + ttl.min(ASC_MAX_TTL),
        "aud": ASC_AUDIENCE,
    });
    format!(
        "{}.{}",
        b64url(header.to_string().as_bytes()),
        b64url(claims.to_string().as_bytes())
    )
}

/// A whole token from its `signing_input` and the raw `r‖s` ES256 signature `WebCrypto` gives.
#[must_use]
pub fn jwt(signing_input: &str, signature: &[u8]) -> String {
    format!("{signing_input}.{}", b64url(signature))
}

/// The DER of a PEM private key such as an App Store Connect `.p8`.
///
/// # Errors
///
/// When the PEM holds no base64 body.
pub fn pem_der(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    if body.is_empty() {
        return Err("the key holds no PEM body".into());
    }
    base64(&body)
}

/// Standard base64, padded or not, decoded; whitespace is ignored.
///
/// # Errors
///
/// On a character outside the alphabet.
pub fn base64(text: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
    {
        let v = ALPHABET
            .iter()
            .position(|&b| b == c)
            .ok_or("the value isn't base64")?;
        acc = (acc << 6) | u32::try_from(v).unwrap_or_default();
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or_default());
        }
    }
    Ok(out)
}

/// The Buildkite job a verified OIDC token speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The build's number in the pipeline.
    pub build: u64,
    /// The commit it builds.
    pub commit: String,
}

/// The claims of a verified Buildkite OIDC token, checked: issuer, audience `audience`, time,
/// and that it is [`PIPELINE`]'s [`STEP`] step in [`ORGANIZATION`] on [`BRANCH`].
///
/// # Errors
///
/// The first claim that disqualifies it.
pub fn check_claims(claims: &Value, audience: &str, now: u64) -> Result<Job, String> {
    let s = |k: &str| claims.get(k).and_then(Value::as_str).unwrap_or_default();
    let n = |k: &str| claims.get(k).and_then(Value::as_u64);
    if s("iss") != BUILDKITE_ISSUER {
        return Err("not a Buildkite token".into());
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == audience,
        Some(Value::Array(a)) => a.iter().any(|v| v.as_str() == Some(audience)),
        _ => false,
    };
    if !aud_ok {
        return Err("the token is for another audience".into());
    }
    match n("exp") {
        Some(exp) if exp + SKEW > now => {}
        _ => return Err("the token has expired".into()),
    }
    if n("nbf").is_some_and(|nbf| nbf > now + SKEW) || n("iat").is_some_and(|iat| iat > now + SKEW)
    {
        return Err("the token is not valid yet".into());
    }
    for (key, want) in [
        ("organization_slug", ORGANIZATION),
        ("pipeline_slug", PIPELINE),
        ("step_key", STEP),
        ("build_branch", BRANCH),
    ] {
        if s(key) != want {
            return Err(format!("`{key}` must be `{want}`"));
        }
    }
    let commit = s("build_commit");
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("`build_commit` must be a full commit SHA".into());
    }
    let build = n("build_number").ok_or("`build_number` is missing")?;
    Ok(Job {
        build,
        commit: commit.to_ascii_lowercase(),
    })
}

/// The release states, in order.
pub const STATES: &[&str] = &[
    "queued",
    "archiving",
    "archived",
    "signing",
    "uploaded",
    "processing",
    "delivered",
    "failed",
];

/// Redeems `job`'s release once: the row for its build and commit, archived and never redeemed,
/// moves to `signing`. It returns the release's build number, or no row when anything differs.
#[must_use]
pub fn redeem(job: &Job, now: u64) -> Statement {
    Statement {
        sql: "UPDATE ios_releases SET state = 'signing', redeemed_at = ?1, updated_at = ?1 \
              WHERE buildkite_build = ?2 AND sha = ?3 AND state = 'archived' \
              AND redeemed_at IS NULL RETURNING build",
        params: vec![json!(now), json!(job.build), json!(job.commit)],
    }
}

/// Marks release `build` failed, with `why`.
#[must_use]
pub fn fail(build: u64, why: &str, now: u64) -> Statement {
    Statement {
        sql: "UPDATE ios_releases SET state = 'failed', error = ?2, updated_at = ?3 WHERE build = ?1",
        params: vec![json!(build), json!(why), json!(now)],
    }
}

/// The App Store provisioning profile for the app, from `GET /v1/bundleIds/{id}/profiles`: the
/// active `IOS_APP_STORE` profile that expires last, as `(uuid, name, content)`.
///
/// # Errors
///
/// When there is none.
pub fn app_store_profile(profiles: &Value) -> Result<(String, String, String), String> {
    profiles["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| &p["attributes"])
        .filter(|a| a["profileType"] == "IOS_APP_STORE" && a["profileState"] == "ACTIVE")
        .max_by(|a, b| {
            a["expirationDate"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["expirationDate"].as_str().unwrap_or_default())
        })
        .and_then(|a| {
            Some((
                a["uuid"].as_str()?.to_string(),
                a["name"].as_str()?.to_string(),
                a["profileContent"].as_str()?.to_string(),
            ))
        })
        .ok_or_else(|| format!("no active App Store profile for {BUNDLE_ID}"))
}

/// The ID of the bundle ID resource for [`BUNDLE_ID`], from `GET /v1/bundleIds?filter[identifier]=…`.
///
/// # Errors
///
/// When it isn't registered.
pub fn bundle_id(answer: &Value) -> Result<String, String> {
    answer["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["attributes"]["identifier"] == BUNDLE_ID)
        .and_then(|d| d["id"].as_str())
        .map(String::from)
        .ok_or_else(|| format!("{BUNDLE_ID} is not registered"))
}

/// Base64 in base64url, for the harness's `Grant`.
///
/// # Errors
///
/// When `text` isn't base64.
pub fn to_b64url(text: &str) -> Result<String, String> {
    base64(text).map(|b| b64url(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUD: &str = "https://platform.pigeon.dev";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn claims() -> Value {
        json!({
            "iss": BUILDKITE_ISSUER,
            "aud": AUD,
            "iat": 1_000,
            "nbf": 1_000,
            "exp": 1_300,
            "organization_slug": ORGANIZATION,
            "pipeline_slug": PIPELINE,
            "step_key": STEP,
            "build_branch": BRANCH,
            "build_commit": SHA,
            "build_number": 7,
        })
    }

    #[test]
    fn a_sign_steps_token_names_its_build_and_commit() {
        assert_eq!(
            check_claims(&claims(), AUD, 1_100),
            Ok(Job {
                build: 7,
                commit: SHA.into()
            })
        );
    }

    #[test]
    fn every_wrong_claim_is_refused() {
        let cases: &[(&str, Value)] = &[
            ("iss", json!("https://evil.example")),
            ("aud", json!("https://elsewhere")),
            ("organization_slug", json!("noisypigeon")),
            ("pipeline_slug", json!("pigeon-ci")),
            ("step_key", json!("archive")),
            ("build_branch", json!("change/1-x")),
            ("build_commit", json!("abc")),
            ("build_number", Value::Null),
            ("exp", json!(900)),
            ("nbf", json!(5_000)),
        ];
        for (key, value) in cases {
            let mut c = claims();
            c[*key] = value.clone();
            assert!(check_claims(&c, AUD, 1_100).is_err(), "{key} = {value}");
        }
        let mut c = claims();
        c.as_object_mut().expect("object").remove("step_key");
        assert!(check_claims(&c, AUD, 1_100).is_err());
    }

    #[test]
    fn an_audience_list_counts_when_it_holds_ours() {
        let mut c = claims();
        c["aud"] = json!(["x", AUD]);
        assert!(check_claims(&c, AUD, 1_100).is_ok());
    }

    #[test]
    fn redeeming_needs_the_archived_unredeemed_row_of_that_build_and_commit() {
        let s = redeem(
            &Job {
                build: 7,
                commit: SHA.into(),
            },
            1_100,
        );
        for part in [
            "state = 'archived'",
            "redeemed_at IS NULL",
            "buildkite_build = ?2",
            "sha = ?3",
            "RETURNING",
        ] {
            assert!(s.sql.contains(part), "{part}");
        }
        assert_eq!(s.params, vec![json!(1_100), json!(7), json!(SHA)]);
    }

    #[test]
    fn asc_tokens_never_outlive_twenty_minutes() {
        let input = asc_jwt_input("KEY123", "issuer-uuid", 1_000, 3_600);
        let (h, c) = input.split_once('.').expect("two parts");
        let header: Value =
            serde_json::from_slice(&crate::push::b64url_decode(h).expect("b64")).expect("json");
        let claims: Value =
            serde_json::from_slice(&crate::push::b64url_decode(c).expect("b64")).expect("json");
        assert_eq!(
            header,
            json!({ "alg": "ES256", "kid": "KEY123", "typ": "JWT" })
        );
        assert_eq!(claims["exp"], 1_000 + ASC_MAX_TTL);
        assert_eq!(claims["aud"], ASC_AUDIENCE);
        assert_eq!(claims["iss"], "issuer-uuid");
    }

    #[test]
    fn a_p8_decodes_and_base64_converts() {
        let pem = "-----BEGIN PRIVATE KEY-----\nAAEC\nAwQ=\n-----END PRIVATE KEY-----\n";
        assert_eq!(pem_der(pem), Ok(vec![0, 1, 2, 3, 4]));
        assert!(pem_der("-----BEGIN-----\n-----END-----").is_err());
        assert_eq!(to_b64url("+/8="), Ok("-_8".into()));
        assert!(base64("not*base64").is_err());
    }

    #[test]
    fn the_latest_active_app_store_profile_is_chosen() {
        let p = |name: &str, kind: &str, state: &str, exp: &str| {
            json!({ "attributes": { "name": name, "uuid": format!("{name}-uuid"),
                "profileType": kind, "profileState": state, "expirationDate": exp,
                "profileContent": "Y29udGVudA==" } })
        };
        let answer = json!({ "data": [
            p("old", "IOS_APP_STORE", "ACTIVE", "2027-01-01T00:00:00.000+0000"),
            p("new", "IOS_APP_STORE", "ACTIVE", "2027-11-01T00:00:00.000+0000"),
            p("dev", "IOS_APP_DEVELOPMENT", "ACTIVE", "2028-01-01T00:00:00.000+0000"),
            p("gone", "IOS_APP_STORE", "INVALID", "2029-01-01T00:00:00.000+0000"),
        ]});
        let (uuid, name, _) = app_store_profile(&answer).expect("one");
        assert_eq!((uuid.as_str(), name.as_str()), ("new-uuid", "new"));
        assert!(app_store_profile(&json!({ "data": [] })).is_err());
    }

    #[test]
    fn the_bundle_is_found_by_identifier() {
        let answer = json!({ "data": [
            { "id": "A1", "attributes": { "identifier": "dev.pigeon.other" } },
            { "id": "B2", "attributes": { "identifier": BUNDLE_ID } },
        ]});
        assert_eq!(bundle_id(&answer), Ok("B2".into()));
        assert!(bundle_id(&json!({ "data": [] })).is_err());
    }

    #[test]
    fn states_run_from_queued_to_delivered() {
        assert_eq!(STATES.first(), Some(&"queued"));
        assert!(STATES.contains(&"archived") && STATES.contains(&"signing"));
    }
}
