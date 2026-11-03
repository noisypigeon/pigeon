# infra

Break-glass access and the basic bootstrapping of zones and providers, per
[`2026-11-02-development-repository-area-layout.md`](../.claude/decisions/2026-11-02-development-repository-area-layout.md).
Each child is a config unit: `cloudflare/` and `digitalocean/` provision their
providers, and `agnostic/` holds provider-agnostic modules any unit composes, per
[`2026-11-03-runtime-infra-agnostic-shared-modules.md`](../.claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md).
