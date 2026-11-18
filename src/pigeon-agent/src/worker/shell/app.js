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
  if (/^\/(approve|mandate|lane|grants|ci-window|platform|pool-resume)\/[^?]+\?grant=/.test(linked)) prove(linked);
  else go(current);
})();
