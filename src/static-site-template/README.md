# static-site-template

pigeon's [Zola](https://www.getzola.org/) theme for static sites on
Cloudflare R2, ported from runway's theme at 1.3.2. Its design is
[`2026-11-03-runtime-cloudflare-static-sites-r2.md`](../../.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md).

A workload's `config.yaml` selects a released version and its features, as
`static_site.template: { version, features }`. When cobot builds the site, it
puts that tag's tree at `themes/static-site-template` and writes
`[extra.features]`. Features left unset are off. [`template.yaml`](template.yaml)
holds the version, the features and their defaults.

## Site config

From 1.1.0, a site's settings live in its workload's `config.yaml`, as
`static_site.site`, per
[`2026-11-03-runtime-static-site-config-contract.md`](../../.claude/decisions/2026-11-03-runtime-static-site-config-contract.md).

- [`schema.json`](schema.json) is the contract: each setting, its
  description and default, and its `x-site` targets, the files and keys
  cobot writes it to.
- [`scaffold/`](scaffold) holds those files' fixed parts: `config.toml`,
  `static/manifest.json`, and the `posts` and `pages` sections.
- `templates/robots.txt` renders `robots.txt` from `base_url`, and
  `index.html` renders `extra.intro` when the home page has no
  `content/_index.md`.

Such a site's `site/` holds only content and assets:

```
site/
  posts/     YYYY-MM-DD-<slug>.md
  pages/
  assets/    served at /assets/
```

`template.yaml`'s `content.posts.filename: date-slug` is the naming rule: a
post's date prefix equals the date of its `date`, and it sets no `slug`.

The posts feed is `/posts/posts_feed.xml`, which the scaffold generates and
`feed` links to.

A site without `static_site.site` supplies its own `config.toml`,
`content/` and `static/`, as in 1.0.0; its own `robots.txt` wins over the
template's.
