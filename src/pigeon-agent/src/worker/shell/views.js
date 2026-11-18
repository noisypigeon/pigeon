// The four views, rendered: the shell's only DOM builder; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.
(() => {
  "use strict";

  const el = (tag, attrs, ...kids) => {
    const n = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === null || v === undefined || v === false) continue;
      if (k === "class") n.className = v;
      else if (k.startsWith("on")) n.addEventListener(k.slice(2), v);
      else n.setAttribute(k, v === true ? "" : String(v));
    }
    for (const k of kids.flat(Infinity)) {
      if (k === null || k === undefined || k === false) continue;
      n.append(k instanceof Node ? k : document.createTextNode(String(k)));
    }
    return n;
  };

  const when = (at) => (at ? new Date(at * 1000).toLocaleString() : "");
  const span = (s) => {
    const d = Math.floor(s / 86400);
    const h = Math.floor((s % 86400) / 3600);
    const m = Math.floor((s % 3600) / 60);
    return d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : m ? `${m}m` : `${s}s`;
  };

  // One field value, by its kind.
  const value = (v) => {
    if (!v || typeof v !== "object") return String(v ?? "");
    switch (v.kind) {
      case "code":
        return el("code", {}, v.text);
      case "sha":
        return el("code", { title: v.text }, String(v.text || "").slice(0, 12));
      case "time":
        return el("span", { title: String(v.at) }, when(v.at));
      case "duration":
        return span(v.seconds || 0);
      case "count":
        return String(v.n);
      case "link":
        return el("a", { href: v.href, rel: "noopener" }, v.text);
      case "progress":
        return el("span", {}, el("progress", { max: v.of || 1, value: v.stop || 0 }), ` ${v.stop}/${v.of} ${v.mark || ""}`);
      case "series":
        return (v.points || []).map((p) => `${p.label} ${p.n}`).join(" · ");
      default:
        return v.text ?? "";
    }
  };

  const fields = (list) =>
    (list || []).length ? el("dl", {}, (list || []).map((f) => [el("dt", {}, f.label), el("dd", {}, value(f.value))])) : null;

  const actions = (list, act) =>
    (list || []).length
      ? el(
          "div",
          { class: "actions" },
          list.map((a) => el("button", { class: a.style === "primary" ? null : a.style || "plain", onclick: () => act(a) }, a.label + (a.proof ? " 🔑" : ""))),
        )
      : null;

  const list = (v, h) =>
    el(
      "section",
      {},
      el("h1", {}, v.title),
      (v.items || []).length
        ? el(
            "ul",
            { class: "rows" },
            v.items.map((r) =>
              el(
                "li",
                { "data-open": !!r.open, onclick: r.open ? () => h.act(r.open) : null },
                el("div", {}, el("strong", {}, r.title), r.subtitle ? el("small", {}, r.subtitle) : null),
                r.badge || r.needs === "owner"
                  ? el("span", { class: r.needs === "owner" ? "badge owner" : "badge" }, r.badge || "needs you")
                  : null,
              ),
            ),
          )
        : el("p", {}, "Nothing here."),
    );

  const card = (v, h) =>
    el("section", {}, el("h1", {}, v.title), v.body ? el("p", {}, v.body) : null, fields(v.fields), actions(v.actions, h.act));

  const detail = (v, h) =>
    el(
      "section",
      {},
      el("h1", {}, v.title),
      (v.sections || []).map((s) =>
        el("div", {}, s.heading ? el("h2", {}, s.heading) : null, fields(s.fields), s.text ? el("pre", {}, s.text) : null),
      ),
      actions(v.actions, h.act),
    );

  const form = (v, h) => {
    const inputs = (v.fields || []).map((f) => {
      const id = `f-${f.key}`;
      let input;
      if (f.input === "choice") input = el("select", { id, name: f.key }, (f.options || []).map((o) => el("option", { value: o }, o)));
      else if (f.input === "toggle") input = el("input", { id, name: f.key, type: "checkbox" });
      else
        input = el("input", {
          id,
          name: f.key,
          type: f.input === "number" ? "number" : f.input === "secret" ? "password" : "text",
          required: !!f.required,
        });
      return el("div", {}, el("label", { for: id }, f.label), input, f.help ? el("div", { class: "sub" }, f.help) : null);
    });
    const submit = (e) => {
      e.preventDefault();
      const args = Object.assign({}, v.submit.arguments || {});
      for (const f of v.fields || []) {
        const n = e.target.elements[f.key];
        if (!n) continue;
        args[f.key] = f.input === "toggle" ? n.checked : n.value;
      }
      h.act(Object.assign({}, v.submit, { arguments: args }));
    };
    return el("form", { onsubmit: submit }, el("h1", {}, v.title), inputs, el("div", { class: "actions" }, el("button", { type: "submit" }, v.submit.label)));
  };

  // Renders view `v` into `into`; `h.act(action)` runs an action's tool.
  const render = (into, v, h) => {
    const by = { list, card, detail, form }[v && v.view];
    into.replaceChildren(by ? by(v, h) : el("p", { class: "error" }, "This answer has no view."));
  };

  const message = (into, text, error) => into.replaceChildren(el("p", { class: error ? "error" : null }, text));

  // The root tabs; `current` is marked.
  const roots = (into, names, current, go) =>
    into.replaceChildren(...names.map((n) => el("button", { "aria-current": n === current ? "page" : null, onclick: () => go(n) }, n)));

  // The owner's proof: what the grant approves, and a passkey button.
  const proof = (into, c, sign, cancel) =>
    into.replaceChildren(
      el(
        "section",
        {},
        el("h1", {}, "Approve with your passkey"),
        fields([
          { label: "operation", value: { kind: "text", text: c.operation } },
          { label: "target", value: { kind: "text", text: c.target } },
          { label: "head", value: { kind: "code", text: c.head || "" } },
        ]),
        el(
          "div",
          { class: "actions" },
          el("button", { onclick: sign }, "Approve 🔑"),
          el("button", { class: "plain", onclick: cancel }, "Cancel"),
        ),
      ),
    );

  window.PigeonViews = { render, message, roots, proof };
})();
