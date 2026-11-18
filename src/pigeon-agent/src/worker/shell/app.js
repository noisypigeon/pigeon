// The shell: roots, navigation and the owner's proof, over the four views; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.3 and §4.
(() => {
  "use strict";
  const { call, pending } = window.PigeonMCP;
  const V = window.PigeonViews;
  const main = document.getElementById("view");
  const nav = document.getElementById("roots");

  const ROOTS = {
    Approvals: ["approval.list", {}],
    Feed: ["feed.read", { needs: "owner", open: "1" }],
    Home: ["home.show", {}],
    Tracker: ["tracker.show", {}],
    Changes: ["change.list", {}],
  };
  let current = "Approvals";

  const b64 = (buf) =>
    btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const bytes = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));

  // The latest answer's WebAuthn options (`challenge`, `rp`, `ids`); see the ADR's 2026-11-18 amendments.
  let held = null;
  const ask = async (tool, args) => {
    const o = await call(tool, args);
    const d = o && o.data;
    if (d && typeof d === "object" && typeof d.challenge === "string" && typeof d.rp === "string") held = d;
    return o;
  };

  // Runs `tool` and shows its answer: its view, the owner's proof when it waits for one, or why it failed.
  const run = async (tool, args) => {
    V.message(main, "Loading…");
    try {
      const o = await ask(tool, args);
      const page = pending(o);
      if (page) return prove(page);
      if (o.view) return V.render(main, o.view, { act });
      V.message(main, o.error || o.text || "No answer.", !!o.error);
    } catch (e) {
      V.message(main, e.message, true);
    }
  };

  const clear = () => {
    location.hash = "";
    held = null;
  };

  // An action: one with `proof` is signed with the held challenge.
  const act = async (a) => {
    if (!a.proof) return run(a.tool, a.arguments || {});
    if (!held) return V.message(main, "No passkey challenge; open the link again.", true);
    if (!window.PublicKeyCredential) return V.message(main, "This browser has no passkeys.", true);
    try {
      const cred = await navigator.credentials.get({
        publicKey: {
          challenge: bytes(held.challenge),
          rpId: held.rp,
          allowCredentials: (held.ids || []).map((id) => ({ type: "public-key", id: bytes(id) })),
          userVerification: "required",
          timeout: 120000,
        },
      });
      const r = cred.response;
      V.message(main, "Sending…");
      const o = await ask(
        a.tool,
        Object.assign({}, a.arguments, {
          credential: cred.id,
          client_data: b64(r.clientDataJSON),
          authenticator_data: b64(r.authenticatorData),
          signature: b64(r.signature),
        }),
      );
      clear();
      if (o.view) return V.render(main, o.view, { act });
      V.message(main, o.error || "Done.", !!o.error);
    } catch (e) {
      V.message(main, e.message, true);
    }
  };

  // The card for the grant at `page`: its view, or, from a worker that sends none, the passkey screen.
  const prove = async (page) => {
    location.hash = page;
    let c;
    try {
      c = await ask("proof.challenge", { page });
    } catch (e) {
      return V.message(main, e.message, true);
    }
    if (c.error || !c.data) return V.message(main, c.error || "No challenge.", true);
    const roots = Object.values(ROOTS).map((r) => r[0]);
    const v = c.view;
    if (v && Array.isArray(v.actions) && v.actions.some((a) => a.proof || !roots.includes(a.tool))) {
      return V.render(main, v, { act });
    }
    const d = c.data;
    const cancel = () => {
      clear();
      go(current);
    };
    const sign = async () => {
      if (!window.PublicKeyCredential) return V.message(main, "This browser has no passkeys.", true);
      try {
        const cred = await navigator.credentials.get({
          publicKey: {
            challenge: bytes(d.challenge),
            rpId: d.rp,
            allowCredentials: (d.ids || []).map((id) => ({ type: "public-key", id: bytes(id) })),
            userVerification: "required",
            timeout: 120000,
          },
        });
        const r = cred.response;
        V.message(main, "Sending…");
        const o = await call("proof.submit", {
          page,
          fields: { operation: d.operation },
          credential: cred.id,
          client_data: b64(r.clientDataJSON),
          authenticator_data: b64(r.authenticatorData),
          signature: b64(r.signature),
        });
        clear();
        if (o.view) return V.render(main, o.view, { act });
        V.message(main, o.error || "Done.", !!o.error);
      } catch (e) {
        V.message(main, e.message, true);
      }
    };
    V.proof(main, d, sign, cancel);
  };

  const go = (name) => {
    clear();
    current = name;
    V.roots(nav, Object.keys(ROOTS), name, go);
    const [tool, args] = ROOTS[name];
    run(tool, args);
  };

  // A link (`/app#/approve/12?grant=…`) opens its proof.
  const linked = decodeURIComponent(location.hash.slice(1));
  V.roots(nav, Object.keys(ROOTS), current, go);
  if (/^\/(approve|mandate|lane|grants|ci-window|platform|pool-resume|service-bind)\/[^?]+\?grant=/.test(linked)) prove(linked);
  else go(current);

  // The Notify button: subscribes this device to Web Push; see the ADR's 2026-11-18 amendment §6.
  const notify = document.getElementById("notify");
  if (notify && "serviceWorker" in navigator && "PushManager" in window) {
    notify.hidden = false;
    notify.addEventListener("click", async () => {
      notify.disabled = true;
      try {
        const reg = await navigator.serviceWorker.register("/sw.js", { scope: "/" });
        await navigator.serviceWorker.ready;
        if ((await Notification.requestPermission()) !== "granted") {
          notify.textContent = "Notifications are blocked";
          return;
        }
        const k = await call("push.key", {});
        const key = k && k.data && k.data.result && k.data.result.key;
        if (!key) throw new Error((k && k.error) || "no key");
        const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: bytes(key) });
        const o = await call("push.subscribe", { endpoint: sub.endpoint });
        if (o.error) throw new Error(o.error);
        notify.textContent = "This device is notified";
      } catch (e) {
        notify.textContent = "Not saved: " + e.message;
        notify.disabled = false;
      }
    });
  }
})();
