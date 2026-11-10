// Passkeys on pigeon's proof pages; see .claude/decisions/2026-11-09-security-approvals-once-passkeys.md §4.
(() => {
  "use strict";
  const b64 = (buf) =>
    btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const bytes = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
  const put = (form, name, value) => {
    let input = form.querySelector(`input[name="${name}"]`);
    if (!input) {
      input = document.createElement("input");
      input.type = "hidden";
      input.name = name;
      form.appendChild(input);
    }
    input.value = value;
  };
  const challenge = async (purpose) => {
    const r = await fetch("/passkeys/challenge", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ purpose }),
    });
    if (!r.ok) throw new Error(await r.text());
    return r.json();
  };
  for (const button of document.querySelectorAll("[data-passkey]")) {
    if (!window.PublicKeyCredential) continue;
    button.hidden = false;
    button.addEventListener("click", async () => {
      const form = button.closest("form");
      const status = form.querySelector("[data-passkey-status]");
      const registering = button.dataset.passkey === "register";
      if (registering && !form.reportValidity()) return;
      try {
        const c = await challenge(button.dataset.purpose);
        const ids = c.ids.map((id) => ({ type: "public-key", id: bytes(id) }));
        if (registering) {
          const cred = await navigator.credentials.create({
            publicKey: {
              challenge: bytes(c.challenge),
              rp: { id: c.rp || location.hostname, name: "pigeon" },
              user: { id: bytes(c.user), name: "owner", displayName: "pigeon owner" },
              pubKeyCredParams: [{ type: "public-key", alg: -7 }],
              authenticatorSelection: { userVerification: "required", residentKey: "preferred" },
              attestation: "none",
              excludeCredentials: ids,
            },
          });
          put(form, "pk_id", cred.id);
          put(form, "pk_client", b64(cred.response.clientDataJSON));
          put(form, "pk_authenticator", b64(cred.response.getAuthenticatorData()));
          put(form, "pk_spki", b64(cred.response.getPublicKey()));
        } else {
          const cred = await navigator.credentials.get({
            publicKey: {
              challenge: bytes(c.challenge),
              rpId: c.rp || location.hostname,
              userVerification: "required",
              allowCredentials: ids,
            },
          });
          put(form, "pk_id", cred.id);
          put(form, "pk_client", b64(cred.response.clientDataJSON));
          put(form, "pk_authenticator", b64(cred.response.authenticatorData));
          put(form, "pk_signature", b64(cred.response.signature));
        }
        form.submit();
      } catch (e) {
        if (status) status.textContent = `Passkey: ${e.message}`;
      }
    });
  }
})();
