# workloads

One directory per Cloudflare zone, holding its workloads. Each zone's apex
(global config and DNS) is its own unit, `<zone>/apex/`, which enables what
`infra/cloudflare/` defines. See
[`2026-11-02-runtime-workloads-zone-apex.md`](../.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md).

A workload with only a `config.yaml` enables a feature `infra/cloudflare/`
defines, such as `static_site`: cobot builds and publishes it. See
[`2026-11-03-runtime-cloudflare-static-sites-r2.md`](../.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md).

A static site's settings live in its `config.yaml`, as `static_site.site`,
and its `site/` holds only `posts/`, `pages/` and `assets/`. See
[`2026-11-03-runtime-static-site-config-contract.md`](../.claude/decisions/2026-11-03-runtime-static-site-config-contract.md).
