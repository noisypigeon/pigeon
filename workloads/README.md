# workloads

One directory per Cloudflare zone, holding its workloads. Every zone's apex
(global config and DNS) is one unit, `cloudflare/apex/`, which enables what
`infra/cloudflare/` defines. See
[`2026-11-02-runtime-workloads-zone-apex.md`](../.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md).
