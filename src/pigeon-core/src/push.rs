//! Web Push to the owner's devices: VAPID-signed pushes with no payload, after which the page's
//! service worker fetches what to show; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §3.

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use serde::{Deserialize, Serialize};

pub use common::base64::{b64url, b64url_decode};

/// How long a push's VAPID token lives, in seconds.
pub const TOKEN_TTL: u64 = 12 * 60 * 60;

/// A VAPID key pair, from its private scalar.
pub struct Vapid(SigningKey);

impl Vapid {
    /// From the `PUSH_VAPID_KEY` secret: the private scalar, base64url.
    ///
    /// # Errors
    ///
    /// When it isn't a valid P-256 scalar.
    pub fn from_secret(secret: &str) -> Result<Self, String> {
        let bytes = b64url_decode(secret.trim())?;
        SigningKey::from_slice(&bytes)
            .map(Self)
            .map_err(|_| "PUSH_VAPID_KEY isn't a P-256 private key".into())
    }

    /// The public key, uncompressed and base64url, as a page's `applicationServerKey`.
    #[must_use]
    pub fn public_key(&self) -> String {
        b64url(self.0.verifying_key().to_encoded_point(false).as_bytes())
    }

    /// The `Authorization` header for a push to `endpoint` at `now`, naming `subject` (a
    /// `mailto:` or `https:` URL).
    ///
    /// # Errors
    ///
    /// When `endpoint` isn't an https URL.
    pub fn authorization(&self, endpoint: &str, subject: &str, now: u64) -> Result<String, String> {
        let header = b64url(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({
            "aud": audience(endpoint)?,
            "exp": now + TOKEN_TTL,
            "sub": subject,
        });
        let body = b64url(claims.to_string().as_bytes());
        let signed = format!("{header}.{body}");
        let sig: Signature = self.0.sign(signed.as_bytes());
        Ok(format!(
            "vapid t={signed}.{}, k={}",
            b64url(&sig.to_bytes()),
            self.public_key()
        ))
    }
}

/// The origin of push endpoint `endpoint`: its scheme and authority.
///
/// # Errors
///
/// When it isn't an https URL.
pub fn audience(endpoint: &str) -> Result<String, String> {
    let rest = endpoint
        .strip_prefix("https://")
        .ok_or("a push endpoint is an https URL")?;
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() {
        return Err("a push endpoint names a host".into());
    }
    Ok(format!("https://{host}"))
}

/// One device subscribed to pushes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// Its push service endpoint.
    pub endpoint: String,
    /// Who subscribed it.
    pub by: String,
    /// When, in Unix seconds.
    pub at: u64,
}

/// The most devices kept.
pub const MAX_DEVICES: usize = 10;

/// Every device subscribed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Devices(pub Vec<Subscription>);

impl Devices {
    /// Adds `endpoint` for `by`, replacing the same endpoint and dropping the oldest past
    /// [`MAX_DEVICES`].
    ///
    /// # Errors
    ///
    /// When `endpoint` isn't an https URL.
    pub fn add(&mut self, endpoint: &str, by: &str, now: u64) -> Result<(), String> {
        audience(endpoint)?;
        self.0.retain(|s| s.endpoint != endpoint);
        self.0.push(Subscription {
            endpoint: endpoint.to_string(),
            by: by.to_string(),
            at: now,
        });
        while self.0.len() > MAX_DEVICES {
            self.0.remove(0);
        }
        Ok(())
    }

    /// Drops `endpoint`, as when its push service says it is gone.
    pub fn remove(&mut self, endpoint: &str) {
        self.0.retain(|s| s.endpoint != endpoint);
    }
}

/// What the newest push says, which the service worker fetches to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    /// The notification's title.
    pub title: String,
    /// Its body.
    pub body: String,
    /// Where tapping it goes, on the control plane.
    pub url: String,
    /// When, in Unix seconds.
    pub at: u64,
}

/// The service worker, served at `/sw.js` with scope `/`: on a push it reads the feed's open owner
/// items over `/mcp` (the ADR's 2026-11-18 amendment §5) and shows their count; a tap opens `/app`.
pub const SERVICE_WORKER: &str = r#"self.addEventListener("push", (event) => {
  event.waitUntil((async () => {
    let body = "Something needs you.";
    try {
      const post = (payload, extra) => fetch("/mcp", {
        method: "POST",
        credentials: "same-origin",
        headers: Object.assign({ "content-type": "application/json" }, extra),
        body: JSON.stringify(payload),
      });
      const init = await post({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "pigeon-sw", version: "1" } } });
      const sid = init.headers.get("mcp-session-id");
      const r = await post({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "feed_read", arguments: { needs: "owner", open: "1" } } }, sid ? { "mcp-session-id": sid } : {});
      if (r.ok) {
        const n = (await r.json()).result._meta["dev.pigeon/view"].items.length;
        body = n === 1 ? "1 needs you" : n + " need you";
      }
    } catch (e) {}
    await self.registration.showNotification("pigeon", { body, data: { url: "/app" }, tag: "pigeon" });
  })());
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(clients.openWindow((event.notification.data && event.notification.data.url) || "/app"));
});
"#;

/// The inbox page's script: subscribes this device on a tap.
pub const PAGE_SCRIPT: &str = r#"document.addEventListener("DOMContentLoaded", () => {
  const b = document.getElementById("notify");
  if (!b) { return; }
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) { b.textContent = navigator.standalone === false ? "Add this page to the Home Screen, then tap here from its icon" : "This browser can't take notifications"; b.disabled = true; return; }
  b.addEventListener("click", async () => {
    b.disabled = true;
    try {
      const key = (await (await fetch("/approvals/vapid", { cache: "no-store" })).text()).trim();
      const pad = "=".repeat((4 - key.length % 4) % 4);
      const raw = atob((key + pad).replace(/-/g, "+").replace(/_/g, "/"));
      const appKey = Uint8Array.from(raw, (c) => c.charCodeAt(0));
      if ((await Notification.requestPermission()) !== "granted") { b.textContent = "Notifications are blocked"; return; }
      const reg = await navigator.serviceWorker.register("/sw.js", { scope: "/" });
      await navigator.serviceWorker.ready;
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: appKey });
      const r = await fetch("/approvals/push", { method: "POST", credentials: "same-origin", redirect: "manual", headers: { "content-type": "application/json" }, body: JSON.stringify({ endpoint: sub.endpoint }) });
      if (r.type === "opaqueredirect") { b.textContent = "Not saved: Access asked to sign in again; reload this page and tap again"; b.disabled = false; return; }
      b.textContent = r.ok ? "This device is notified" : "Not saved: " + r.status + " " + (await r.text()).slice(0, 200);
    } catch (e) { b.textContent = "Not saved: " + e; b.disabled = false; }
  });
});
"#;
