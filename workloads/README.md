# workloads

One directory per Cloudflare zone, holding its workloads. Each zone's apex
(global config and DNS) is its own unit, `<zone>/apex/`, which enables what
`infra/cloudflare/` defines. See
[`2026-11-02-runtime-workloads-zone-apex.md`](../.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md).

A workload with only a `config.yaml` enables a feature `infra/cloudflare/`
defines, such as `static_site`: its Zola site sits in `site/`, and cobot
builds and publishes it. See
[`2026-11-03-runtime-cloudflare-static-sites-r2.md`](../.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md).
