## Static site config from config.yaml, by a template contract

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`2026-11-03-runtime-cloudflare-static-sites-r2.md` moved the blog to `workloads/noisypigeon.com/blog/` as a whole Zola project: `site/config.toml`, `site/content/` and `site/static/`. cobot copies that project into `.cobot/<unit>/site/`, writes `base_url` and `[extra.features]` into its `config.toml`, adds the template as its theme, and runs Zola.

That layout says the same things in several places:

- the title, `Willow Graysen`, is in `config.toml`, `static/manifest.json` and `content/_index.md`;
- the description is in `config.toml` and `static/manifest.json`;
- the domain is hardcoded in `static/robots.txt`, although cobot already derives `base_url` from the apex;
- the posts feed is named in `config.toml`, enabled in `content/posts/_index.md`, and switched by the template's `feed` feature;
- the site's settings are split between the workload's `config.yaml` and `site/config.toml`.

The blog also carries a changelog it no longer wants:

- `content/changelog/0001-…0004-*.md` are four archive posts, published under `/changelog/` with `aliases = ["posts/<slug>"]`;
- `content/changelog/2026-*-changelog.md` are release notes for runway's PRs, carried over with the content. Nothing in this repository writes them.

Posts are named by a sequential count, `0001-<slug>.md` to `0053-<slug>.md`. The count carries nothing that the date does not. Checked on `main`:

- every post's file name, after its count, equals its `slug`;
- no two posts or archive posts share a date, and every `date` is in `+00:00`.

This ADR:

- makes the workload's `config.yaml` the one place for a site's settings;
- gives the template a versioned contract for turning those settings into Zola's files;
- reduces `site/` to content and assets;
- turns the blog's changelog off, and names posts by date.

It amends `2026-11-03-runtime-cloudflare-static-sites-r2.md`, by a dated entry that points here.

## Implementation plan.

### 1. The contract is the template's

The template, `src/static-site-template/`, owns the shape of a site's settings and where each one lands. A released tag fixes both, as it fixes the features today.

- **`schema.json`**: a JSON Schema for `static_site.site`. Each property carries an `x-site` annotation, a list of `{ file, key }` targets. A value can feed several files:

  ```json
  "title": {
    "type": "string",
    "x-site": [
      { "file": "config.toml", "key": "title" },
      { "file": "static/manifest.json", "key": "name" }
    ]
  }
  ```

  - `file` is one of `config.toml` and `static/manifest.json`;
  - `key` is a dotted path, e.g. `extra.author.page`;
  - a property without `x-site` is rejected when the template is released, by the template's own test.
- **`scaffold/`**: the files' fixed parts, which no site sets:
  - `config.toml`: `theme`, `compile_sass`, `generate_feeds`, `feed_filenames`, `build_search_index` and the `tags` taxonomy;
  - `static/manifest.json`: `start_url`, `display`, and `theme_color` and `background_color` from the light theme colour;
  - `content/posts/_index.md`: `sort_by`, `page_template`, and `generate_feeds = true`;
  - `content/pages/_index.md`: `render = false`.
- **`template.yaml`** gains `content: { posts: { filename: date-slug } }`, the rule in §4.

**`infra/cloudflare` invokes it, but does not own it.**

- `infra/cloudflare/schema.json`'s `$defs.workload.static_site` gains one property, `"site": { "type": "object" }`.
- `pigeon cobot validate` validates `static_site.site` against the pinned tag's `schema.json`. It reads that file with `git show <tag>:src/static-site-template/schema.json`, as it reads `template.yaml` for the features.
- A template change therefore never touches `infra/cloudflare/schema.json`, and a site moves to a new contract by bumping `template.version`.

### 2. A site's settings

The blog's `config.yaml` becomes:

```yaml
# The blog, a static site on R2; see .claude/decisions/2026-11-03-runtime-static-site-config-contract.md.
static_site:
  template:
    version: 1.1.0
    features: [math]
  site:
    title: Willow Graysen
    short_name: noisypigeon
    description: Infra dev, data hoarder, mechanic-ish, photographer.
    intro: Hi, I’m Willow Graysen. This is my digital shoebox — equal parts time capsule, portfolio, and proof I was here.
    avatar: /assets/images/site/avatar.png
    social_preview: /assets/images/site/social_preview.png
    author:
      alternate_names: [Willow Finch, Kara Finch, Kara Graysen, Luna Graysen]
      page: /pages/about
    nav:
      - { title: About, link: /pages/about }
      - { title: GitHub, link: "https://github.com/noisypigeon" }
      - { title: Bluesky, link: "https://bsky.app/profile/noisypigeon.com" }
      - { title: LinkedIn, link: "https://www.linkedin.com/in/noisypigeon" }
      - { title: Unsplash, link: "https://unsplash.com/@noisypigeon" }
    icons:
      - { src: /assets/images/site/icon-192.png, sizes: 192x192, type: image/png }
      - { src: /assets/images/site/icon-512.png, sizes: 512x512, type: image/png }
```

- `nav` is ordered by the list, and its `order` keys go.
- `short_name` and `icons` land only in `static/manifest.json`; `intro` lands at `config.toml`'s `extra.intro`.
- The README's site-config table moves into `schema.json`'s descriptions, and the README points there.

### 3. What the template renders

- **`templates/robots.txt`** overrides Zola's built-in one:

  ```
  User-agent: *
  Allow: /

  Sitemap: {{ config.base_url }}/sitemap.xml
  ```

- **`index.html`** renders `section.content` when the root section has a body, and otherwise `config.extra.intro | markdown`. A site then needs no `content/_index.md`.
- These additions, with §1, release `static-site-template/v1.1.0`. A site on `1.0.0` builds as before.

### 4. Posts are named by date

When the pinned template declares `content.posts.filename: date-slug`:

- every file in `posts/` but `_index.md` is named `YYYY-MM-DD-<slug>.md`, matching `^[0-9]{4}-[0-9]{2}-[0-9]{2}-[a-z0-9]+(-[a-z0-9]+)*\.md$`;
- its date prefix equals the date part of its front matter's `date`, as written;
- its front matter sets no `slug`. Zola takes the slug from the name after the date, so `/posts/<slug>/` URLs do not change;
- `date` stays in front matter, because it holds the time.

`pigeon cobot validate` and `pigeon cobot sites build` both check this, and each failure names the file.

### 5. The layout, and the build

A site that sets `static_site.site` holds, under `site/`, exactly:

```
site/
  posts/     the posts, as §4 names them
  pages/     the pages
  assets/    images and other files, served at /assets/
```

`pigeon cobot sites build` assembles `.cobot/<unit>/site/` from it:

1. It copies the tag's `scaffold/`.
2. It writes each `site` value at each of its `x-site` targets.
3. It writes `base_url` and `[extra.features]`, as today.
4. It copies `site/posts/` and `site/pages/` into `content/`, and `site/assets/` into `static/assets/`.
5. It adds the theme from the tag and runs Zola, as today.

Further:

- The build rejects a `site/` holding anything else: in particular `config.toml`, `content/`, `static/`, `robots.txt`, `manifest.json` or an `_index.md`.
- `config.toml` is parsed and written as TOML, through a new `toml` workspace dependency, instead of being spliced line by line. A site still never sets `base_url` or `extra.features`.
- A workload without `static_site.site` keeps today's layout and build until the blog moves. §8's step 5 then removes that path, and `static_site.site` becomes required.

### 6. The blog

Under `workloads/noisypigeon.com/blog/`:

- **The changelog is off.**
  - `changelog` leaves `static_site.template.features`. The template keeps the feature, off by default, for other sites.
  - `content/changelog/0001-…0004-*.md` move to `site/posts/YYYY-MM-DD-<slug>.md`. Each one's alias becomes `changelog/<slug>`, so its old URL redirects to `/posts/<slug>/`.
  - `content/changelog/2026-*-changelog.md` and `content/changelog/_index.md` are deleted. `/changelog/` is gone.
- **Posts are named by date.** `content/posts/NNNN-<slug>.md` move to `site/posts/YYYY-MM-DD-<slug>.md`, without `slug`. `0038`'s alias stays.
- **Pages move.** `content/pages/about.md` and `lineage.md` move to `site/pages/`.
- **Assets move.** `static/assets/images/**` move to `site/assets/images/**`, byte for byte.
- **Generated files are deleted:** `config.toml`, `static/robots.txt`, `static/manifest.json`, and the `_index.md` files of `content/`, `content/posts/` and `content/pages/`. Their values are in `config.yaml`, or in the scaffold.

Every move is a `git mv`, so each file's history follows it.

Out of scope, each for a later change:

- `icon-192.png`, `icon-512.png` and `apple-touch-icon.png` do not exist under `assets/images/site/`. Their references are kept as they are.
- Archive notes link to `/pages/names/`, which does not exist.
- Removing the template's `changelog` feature and `changelog.html`.

### 7. Docs

- `src/static-site-template/README.md` describes `schema.json`, `scaffold/`, the `site/` layout and the `date-slug` rule.
- `CLAUDE.md`'s static-site pointer cites this ADR.
- `workloads/README.md` notes that a static site's settings live in its `config.yaml`.

### 8. Execution

Each step merges, and releases what it changes, before the next one uses it, per `2026-11-03-release-cobot-image-in-workflows.md`.

1. **This PR**, `docs(adr): static site config from config.yaml, by a template contract`, holds this ADR and the static-sites ADR's amendment.
2. **The template PR**, `feat(static-site-template): a site config contract and dated posts`, holds:
   - §1's `schema.json`, `scaffold/` and `template.yaml` key, and the template test that every property has `x-site`;
   - §3's `robots.txt` and `index.html`;
   - §7's README.

   Its merge releases `static-site-template/v1.1.0`.
3. **The cobot PR**, `feat(cobot): build static sites from config.yaml`, holds:
   - §1's `site` property in `infra/cloudflare/schema.json`, and its validation against the tag;
   - §4's naming check;
   - §5's build, and the `toml` dependency;
   - §7's `CLAUDE.md` and `workloads/README.md`.

   Its plan shows "No changes". Its merge releases cobot.
4. **The blog PR**, `feat(workloads): the blog's config from config.yaml`, holds §6 and moves the blog to `1.1.0`. It edits no `CHANGELOG.md`.
5. **The cleanup PR**, `refactor(cobot): only the site config contract`, removes the old layout's build and makes `static_site.site` required.

### 9. Verification

- **Template PR:**
  - `mise run ci` passes;
  - Zola 0.23.6 builds a fixture site from `scaffold/`, rendering the intro and `robots.txt`.
- **cobot PR:**
  - unit tests cover:
    - `x-site` writes into TOML and into JSON, including one value written to two files;
    - a `site` block that breaks the tag's schema fails, with that schema's message;
    - a post without a date prefix, and one whose prefix disagrees with its `date`, each fail;
    - a post that sets `slug` fails;
    - a leftover `site/config.toml` fails;
    - a workload without `static_site.site` still builds the old way;
  - `mise run ci` passes.
- **Blog PR:**
  - `pigeon cobot sites build workloads/noisypigeon.com/blog` passes.
  - Its `public/`, diffed against `main`'s build, changes only:
    - `/changelog/` and the release notes, which are gone;
    - the four archive posts, now under `/posts/`, with `/changelog/<slug>/` redirecting;
    - the footer's changelog link, which is gone.
  - Every `/posts/<slug>/` and `/pages/<slug>/` URL is unchanged.
  - `robots.txt` and `manifest.json` equal today's, ignoring whitespace and key order.
  - `cobot.yml`'s plan shows "No changes".
- **After the blog PR merges:** `publish` deletes the stale `/changelog/` keys, and every live check passes.
