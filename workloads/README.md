# workloads

One directory per Cloudflare zone, each with a required `apex/` unit holding
the zone's global config and DNS; `.apex/` holds the schema, modules and
presets every apex shares. See
[`2026-11-02-runtime-workloads-zone-apex.md`](../.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md).
