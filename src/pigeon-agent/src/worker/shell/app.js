// The shell: roots, navigation and the owner's proof, over the four views; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.3 and §4.
(() => {
  "use strict";
  const { call, pending } = window.PigeonMCP;
  const V = window.PigeonViews;
  const main = document.getElementById("view");
  const nav = document.getElementById("roots");

  const ROOTS = {
    Feed: ["feed.read", { needs: "owner", open: "1" }],
    Home: ["home.show", {}],
    Tracker: ["tracker.show", {}],
    Changes: ["change.list", {}],
  };
  let current = "Feed";

  const b64 = (buf) =>
    btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const bytes = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));

  // Runs `tool` and shows its answer: its view, the owner's proof when it waits for one, or why it failed.
  const run = async (tool, args) => {
    V.message(main, "Loading…");
    try {
      const o = await call(tool, args);
      const page = pending(o);
      if (page) return prove(page);
      if (o.view) return V.render(main, o.view, { act });
      V.message(main, o.error || o.text || "No answer.", !!o.error);
    } catch (e) {
      V.message(main, e.message, true);
    }
  };

  const act = (a) => run(a.tool, a.arguments || {});

  // The proof for the grant at `page`: what it approves, then the passkey, then its outcome.
  const prove = async (page) => {
    location.hash = page;
    let c;
    try {
      c = await call("proof.challenge", { page });
    } catch (e) {
      return V.message(main, e.message, true);
    }
    if (c.error || !c.data) return V.message(main, c.error || "No challenge.", true);
    const d = c.data;
    const cancel = () => {
      location.hash = "";
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
        location.hash = "";
        if (o.view) return V.render(main, o.view, { act });
        V.message(main, o.error || "Done.", !!o.error);
      } catch (e) {
        V.message(main, e.message, true);
      }
    };
    V.proof(main, d, sign, cancel);
  };

  const go = (name) => {
    current = name;
    V.roots(nav, Object.keys(ROOTS), name, go);
    const [tool, args] = ROOTS[name];
    run(tool, args);
  };

  // A link already sent (`/app#/approve/12?grant=…`) opens its proof.
  const linked = decodeURIComponent(location.hash.slice(1));
  V.roots(nav, Object.keys(ROOTS), current, go);
  if (/^\/(approve|mandate|lane|grants|ci-window|platform)\/[^?]+\?grant=/.test(linked)) prove(linked);
  else go(current);
})();
