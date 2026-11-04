# infra

Break-glass access and the basic bootstrapping of zones and providers, per
[`2026-11-02-development-repository-area-layout.md`](../.claude/decisions/2026-11-02-development-repository-area-layout.md).
Each child is a config unit that uses its provider's catalog in `../catalog/`:
`cloudflare/` provisions Cloudflare, and `agnostic/` holds no resources of its
own, per
[`2026-11-04-development-platform-instance-separation.md`](../.claude/decisions/2026-11-04-development-platform-instance-separation.md).
