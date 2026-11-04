# infra

Break-glass access and the basic bootstrapping of zones and providers, per
[`2026-11-02-development-repository-area-layout.md`](../.claude/decisions/2026-11-02-development-repository-area-layout.md).
Each child is a config unit built from the catalogs in `../catalog/`:
`cloudflare/` provisions the instance's Cloudflare, and `pigeon/` bootstraps the
platform's own account, per
[`2026-11-04-development-platform-instance-separation.md`](../.claude/decisions/2026-11-04-development-platform-instance-separation.md).
