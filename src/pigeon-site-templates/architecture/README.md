# site-template-architecture

pigeon's architecture diagram as a [Zola](https://www.getzola.org/) theme, per
[`2026-11-05-runtime-architecture-site-through-backplane.md`](../../../.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md) §3.
Tagged `site-template-architecture/vX.Y.Z`.

- `content: none`: a site using it has no `site/`.
- The page is the network diagram: it draws `data/architecture.json`'s `network`,
  which `pigeon cobot sites build --data architecture=FILE` writes from
  `pigeon backplane run architecture.describe`, per the ADR's 2026-11-11
  amendment. It only draws; every box, route and label comes from `describe`.
- [`rail.js`](static/assets/js/rail.js) routes the dashed line back from GitHub;
  without it, the GitHub card says the same in words.
- [`schema.json`](schema.json) is the contract for `static_site.site`: `title`
  and `description`.
