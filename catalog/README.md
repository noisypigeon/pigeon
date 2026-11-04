# catalog

Provider catalogs: each `catalog/<provider>/` holds a provider's `schema.json`,
single-resource `modules/`, `presets/` and `capabilities.yaml`. Catalogs are
product; units that use them live in `infra/` and `workloads/`. See
[`2026-11-04-development-platform-instance-separation.md`](../.claude/decisions/2026-11-04-development-platform-instance-separation.md).
