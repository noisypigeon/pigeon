//! Cloudflare Access in front of the control plane (control-plane ADR §9): the pure parts of
//! checking a `Cf-Access-Jwt-Assertion`, shared by both builds. The Worker verifies the RS256
//! signature with `WebCrypto` against the key [`key`] picks; everything else is here and tested
//! natively.

use serde_json::Value;

use crate::wire::{Effect, Operation};

/// The header Access puts its token in.
pub const HEADER: &str = "Cf-Access-Jwt-Assertion";

/// A decoded, not yet verified, token.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// Its header.
    pub header: Value,
    /// Its claims.
    pub claims: Value,
    /// `<header>.<claims>`, as signed.
    pub signing_input: String,
    /// The signature's bytes.
    pub signature: Vec<u8>,
}

/// Who a verified token names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// A person's email, or a service token's client ID.
    pub subject: String,
    /// Whether it is a person, signed in; a service token is not.
    pub human: bool,
}

/// Decodes base64url without padding.
///
/// # Errors
///
/// A message for a character outside the alphabet.
pub fn base64url(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err("not base64url".into()),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or_default());
        }
    }
    Ok(out)
}

/// Decodes a compact JWS.
///
/// # Errors
///
/// A message when it isn't three base64url parts, or its header or claims aren't JSON
/// objects.
pub fn decode(jwt: &str) -> Result<Token, String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    let [h, c, sig] = parts.as_slice() else {
        return Err("the token is not a JWS".into());
    };
    let json = |p: &str, what: &str| {
        base64url(p)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(Value::is_object)
            .ok_or_else(|| format!("the token's {what} is not a JSON object"))
    };
    Ok(Token {
        header: json(h, "header")?,
        claims: json(c, "claims")?,
        signing_input: format!("{h}.{c}"),
        signature: base64url(sig)?,
    })
}

/// The JWK in `keys`, a JWKS, that signed `token`: RS256 only, by `kid`.
///
/// # Errors
///
/// A message when the token isn't RS256 or no key has its `kid`.
pub fn key<'a>(token: &Token, keys: &'a Value) -> Result<&'a Value, String> {
    if token.header["alg"] != "RS256" {
        return Err("the token is not RS256".into());
    }
    let kid = token.header["kid"]
        .as_str()
        .ok_or("the token names no key")?;
    keys["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["kid"] == kid && k["kty"] == "RSA")
        .ok_or_else(|| format!("no Access key `{kid}`; the keys may have rotated"))
}

/// The team's issuer: `https://<team>.cloudflareaccess.com`.
#[must_use]
pub fn issuer(team: &str) -> String {
    format!("https://{team}.cloudflareaccess.com")
}

/// Who `token` names, once its claims hold for application `aud` of `team` at `now`.
///
/// # Errors
///
/// A message naming the claim that failed.
pub fn identity(token: &Token, team: &str, aud: &str, now: u64) -> Result<Identity, String> {
    let c = &token.claims;
    if c["iss"].as_str() != Some(issuer(team).as_str()) {
        return Err("the token is from another Access team".into());
    }
    let audiences: Vec<&str> = match &c["aud"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !audiences.contains(&aud) {
        return Err("the token is for another Access application".into());
    }
    let exp = c["exp"].as_u64().ok_or("the token has no expiry")?;
    if exp <= now {
        return Err("the token has expired".into());
    }
    if c["nbf"].as_u64().is_some_and(|nbf| nbf > now + 60) {
        return Err("the token is not valid yet".into());
    }
    if let Some(email) = c["email"].as_str().filter(|e| e.contains('@')) {
        return Ok(Identity {
            subject: email.to_string(),
            human: true,
        });
    }
    c["common_name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| Identity {
            subject: s.to_string(),
            human: false,
        })
        .ok_or_else(|| "the token names no one".into())
}

/// Whether `who` may run `op`: anyone Access lets in reads, plans and builds; only a person
/// applies, which covers approving a plan or a change, closing one, and applying infra.
///
/// # Errors
///
/// A message saying who may.
pub fn authorize(op: &Operation, who: &Identity) -> Result<(), String> {
    if op.effect == Effect::Apply && !who.human {
        return Err(format!(
            "`{}` is for a person signed in to Access, not a service token",
            op.name
        ));
    }
    Ok(())
}
