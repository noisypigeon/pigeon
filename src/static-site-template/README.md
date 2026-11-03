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

| Key | Default | Use |
| --- | --- | --- |
| `extra.avatar`, `extra.social_preview`, `extra.nav` | none | header, Open Graph and nav |
| `extra.author.name` | `config.title` | JSON-LD and byline |
| `extra.author.alternate_names` | none | JSON-LD `alternateName` |
| `extra.author.page` | none | byline link |
| `extra.labels.back`, `extra.labels.subscribe` | `Back to all posts`, `Subscribe via RSS` | post and page footers |
| `extra.look.color`, `extra.look.font` | `gameboy`, `ibm-plex` | `color-theme-*` and `font-theme-*` classes |
| `extra.look.font_family` | IBM Plex Sans | Bunny Fonts family, with `fonts` |
| `extra.theme_color.light`, `.dark` | the gameboy colours | `theme-color` meta |
| `extra.og_image_size.width`, `.height` | `1188`, `670` | Open Graph image size |
| `extra.banner.message`, `.link`, `.link_text` | none | the banner, with `banner` |

`feed` links to `/posts/posts_feed.xml`. That file exists only when the site
sets `feed_filenames = ["posts_feed.xml"]` in `config.toml` and
`generate_feeds = true` in `content/posts/_index.md`.

A site supplies its own `content/`, its static files (icons, `manifest.json`,
`robots.txt`, images), and any template it overrides.
