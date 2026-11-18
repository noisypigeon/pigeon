// The shell's MCP client: JSON-RPC to /mcp, same-origin; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §1.
(() => {
  "use strict";
  let session = null;
  let next = 1;

  const post = async (method, params) => {
    const headers = { "content-type": "application/json", accept: "application/json" };
    if (session) headers["mcp-session-id"] = session;
    const r = await fetch("/mcp", {
      method: "POST",
      credentials: "same-origin",
      headers,
      body: JSON.stringify({ jsonrpc: "2.0", id: next++, method, params }),
    });
    if (r.status === 404 && session) {
      session = null;
      throw new Error("session expired");
    }
    if (!r.ok) throw new Error(`${r.status} ${(await r.text()).slice(0, 200)}`);
    const id = r.headers.get("mcp-session-id");
    if (id) session = id;
    const body = await r.json();
    if (body.error) throw new Error(body.error.message || "error");
    return body.result;
  };

  const start = async () => {
    if (session) return;
    await post("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "pigeon-web", version: "1" },
    });
  };

  // A tool's outcome: its view, its structured result, and whether it failed.
  const call = async (tool, args) => {
    await start();
    let result;
    try {
      result = await post("tools/call", { name: tool.replace(/\./g, "_"), arguments: args || {} });
    } catch (e) {
      if (e.message !== "session expired") throw e;
      await start();
      result = await post("tools/call", { name: tool.replace(/\./g, "_"), arguments: args || {} });
    }
    const text = (result.content || []).map((c) => c.text || "").join("\n");
    return {
      view: result._meta && result._meta["dev.pigeon/view"],
      data: result.structuredContent,
      error: result.isError ? text : null,
      text,
    };
  };

  // The approval link an owner's operation answered with, as a path; null when there is none.
  const pending = (outcome) => {
    const d = outcome && outcome.data;
    let link = d && d.result && d.result.pending && d.result.pending.link;
    if (!link && outcome && outcome.text) {
      const m = outcome.text.match(/https:\/\/[^\s"]+\/(approve|mandate|lane|grants|ci-window|platform)\/[^\s"]+/);
      link = m && m[0];
    }
    if (!link) return null;
    try {
      const u = new URL(link);
      return u.pathname + u.search;
    } catch (_) {
      return null;
    }
  };

  window.PigeonMCP = { call, pending };
})();
