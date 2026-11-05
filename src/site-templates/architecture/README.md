# site-template-architecture

pigeon's architecture diagram as a [Zola](https://www.getzola.org/) theme, per
[`2026-11-05-runtime-architecture-site-through-backplane.md`](../../../.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md) §3.
Tagged `site-template-architecture/vX.Y.Z`.

- `content: none`: a site using it has no `site/`.
- The page draws `data/architecture.json`, which `pigeon cobot sites build --data
  architecture=FILE` writes from `pigeon backplane run architecture.describe`.
  It only draws; every position comes from backplane's layout.
- [`schema.json`](schema.json) is the contract for `static_site.site`: `title`
  and `description`.
