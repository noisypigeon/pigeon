## Static sites on Cloudflare R2, and the site template

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md` and its amendment give each zone an apex, served by one unit, `workloads/cloudflare/apex/`. That apex is assigned to `blog`, which does not exist yet. Its out-of-scope list left the blog to #32. This ADR builds what the blog needs, and the blog itself.

runway already serves the blog and the garden from Cloudflare R2. That design works, and it is the model here. Nothing is copied: every module and template is written fresh in pigeon's shape.

How runway does it:

- **Resources.** A workload's `deployments: { cloudflare: {} }` expands to:
  - an EU-jurisdiction R2 bucket;
  - one `cloudflare_r2_custom_domain` per host, `@` and `www` for an apex site;
  - a token scoped to that bucket, for uploads;
  - two rulesets in the zone: an `http_request_transform` rule that serves `/` from `/index.html`, and an `http_request_dynamic_redirect` that sends `www` to the apex with a 301.
- **What it leaves out.** It uses no Workers, no cache or 404 rules, and no separate DNS records, because a custom domain creates its own.
- **Build.** Zola 0.23.6 builds the site.
- **Upload.**
  - It uses R2's S3 API, at `https://<account>.eu.r2.cloudflarestorage.com`.
  - The access key is the token's id, and the secret is the SHA-256 of its value.
  - Every `x/index.html` is also written as `x` and `x/`, so a directory URL resolves with or without its slash.
  - Stale keys are deleted.
  - Live checks then probe the site.
- **Theme.** runway's Zola theme, `tools/static-site/zola` at 1.3.2, is GPL-3.0, like pigeon. Sites pin it by a git tag. It hardcodes runway's details:
  - the author's names and byline link;
  - its labels;
  - its colour and font classes;
  - KaTeX and Bunny Fonts from CDNs;
  - a commit SHA in the footer that the R2 build never sets.

Pigeon decides it differently in five places:

- **The workload owns its site.** `workloads/<zone>/<workload>/` creates its bucket and custom domains with a token `infra/cloudflare` mints for it. It writes no Cloudflare definitions of its own: it enables a feature that `infra/cloudflare` defines, exactly as the apex does.
- **The zone's rules come from the apex.** A zone has one ruleset per phase, so a workload cannot own one. The apex renders each zone's two rulesets from the static sites assigned to it. It renders no DNS record for their hosts, because the custom domains own DNS there.
- **Only cobot uploads.** The upload credential lives in `infra/cloudflare`'s encrypted state, so only cobot can read it. cobot uploads with its own S3 client, without an external CLI.
- **The template is a project.** It lives in `src/static-site-template/` and is released and tagged like a crate. Consumers pin a released version and enable its features by name.
- **The blog moves here.** Its content moves from runway to `workloads/noisypigeon.com/blog/site/`, and #32 closes.

This ADR amends:

- the cloudflare ADR, `2026-11-02-runtime-cloudflare-provisioner-unit.md`, for site tokens and the account-id output;
- the workloads ADR's amendment, for the apex token's permissions and for static-site hosts.

Each amendment is appended as a dated entry that points here.

Out of scope, each for a later ADR or amendment:

- the garden, and any site other than the blog;
- a custom 404 page and cache rules;
- search;
- private sites;
- scoping a site's upload token to its bucket. Cloudflare only scopes a token to a bucket that already exists, and `infra/cloudflare` applies before workloads.

## Implementation plan.

### 1. Modules

Each lives in `infra/cloudflare/modules/` and wraps one resource, in the unified module shape, pinned to cloudflare 5.28.0. The arguments were checked against that version's provider schema.

| Module | Resource | Inputs | Outputs |
| --- | --- | --- | --- |
| `r2-bucket` | `cloudflare_r2_bucket` | `account_id`, `name`, `jurisdiction` (default `eu`), `storage_class` (default `Standard`) | `id`, `name` |
| `r2-custom-domain` | `cloudflare_r2_custom_domain` | `account_id`, `bucket_name`, `jurisdiction` (default `eu`), `domain`, `zone_id`, `enabled` (default `true`), `min_tls` (default `1.2`) | `id`, `name` |
| `ruleset` | `cloudflare_ruleset` | `zone_id`, `name`, `phase`, `rules`; `kind` is fixed to `zone` | `id`, `name` |

- Each is a release unit, `infra-cloudflare-<module>`, with its own changelog tier. `release-changelog.yaml` lists the tiers.
- The bucket's jurisdiction is EU unless a later amendment says otherwise, as in runway.

### 2. The workload extension of `infra/cloudflare/schema.json`

**Which units use it.** A unit at `workloads/<zone>/<workload>/` with a `config.yaml` and no `schema.json` validates against `infra/cloudflare/schema.json`'s `$defs.workload`. It is composed by `x-cobot.workload`, and its modules come from `infra/cloudflare/`. This is how the apex uses `$defs.apex`. A workload with its own `schema.json` is unchanged.

**The one feature for now is `static_site`:**

```yaml
# The blog; see .claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md.
static_site:
  template:
    version: 1.0.0
    features: [feed, tags, changelog, math, fonts, commit]
  storage_class: Standard
```

`$defs.workload`:

```json
"workload": {
  "description": "A workload under workloads/<zone>/; see .claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md.",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "static_site": {
      "type": "object",
      "additionalProperties": false,
      "required": ["template"],
      "properties": {
        "template": {
          "type": "object",
          "additionalProperties": false,
          "required": ["version"],
          "properties": {
            "version": { "type": "string", "pattern": "^[0-9]+\\.[0-9]+\\.[0-9]+$" },
            "features": {
              "type": "array",
              "uniqueItems": true,
              "items": { "type": "string", "pattern": "^[a-z0-9]+(-[a-z0-9]+)*$" }
            }
          }
        },
        "storage_class": { "enum": ["Standard", "InfrequentAccess"] }
      }
    }
  }
}
```

`x-cobot.workload`:

```json
"workload": {
  "handoff": "workload-token",
  "providers": { "cloudflare": { "api_token": "$handoff.api_token" } },
  "derived": {
    "bucket": {
      "expand": "site-bucket",
      "module": "r2-bucket",
      "inputs": {
        "account_id": "$handoff.account_id",
        "name": "$item.name",
        "storage_class": "$item.storage_class"
      }
    },
    "domains": {
      "expand": "site-domains",
      "module": "r2-custom-domain",
      "inputs": {
        "account_id": "$handoff.account_id",
        "bucket_name": "$item.bucket",
        "domain": "$item.domain",
        "zone_id": "$handoff.zone_id"
      }
    }
  }
}
```

**The expanders:**

- `site-bucket` yields one item, `site`, when `static_site` is set. The bucket is named `<workload>-<zone, dots as dashes>`, e.g. `blog-noisypigeon-com`. The name is deterministic, so `publish` can find the bucket without reading state.
- `site-domains` yields one item for each host the apex assigns to this workload, keyed as the apex keys its host records: `apex` for `@`, and dashes for dots. Each item's `domain` is the host's fully qualified name.
- A workload with `static_site` but no assigned host is rejected.

**Rejected at validate time:**

- `x-cobot.workload` outside `infra/cloudflare/schema.json`;
- a `features` entry the template's `template.yaml` does not declare, checked against the pinned version's tag;
- `static_site` together with `ingress` on the same workload.

### 3. Tokens and the handoff

**Minting.** `infra/cloudflare/config.yaml`'s `tokens` gains one `site-<zone>-<workload>` per static site, holding exactly:

- `Workers R2 Storage Write`, on the account;
- `Zone Read` and `DNS Write`, on that zone. A custom domain needs both.

`validate` checks each site token, as it checks `apex-<zone>`.

**`handoff: workload-token`.** It hands off three values, through `TF_VAR_handoff` only, never on disk:

- `api_token`, the live generation of `site-<zone>-<workload>`;
- `zone_id`, from `infra/cloudflare`'s `zone_ids` output;
- `account_id`, from a new sensitive `infra/cloudflare` output, `account_id`.

A pending handoff plans with placeholders, and an apply refuses, as for the apex.

**The account id.** It is an identifier, not a credential: it grants nothing alone. The cloudflare ADR keeps it in secrets so that it never lands in this public repository. That still holds: it reaches a workload only from encrypted state, through the `tofu` process's environment. It is never a key in a workload's `secrets.config.yaml`, where `account_id` stays rejected.

**The apex token.** `apex-<zone>` holds `DNS Write`, plus `Transform Rules Write` and `Dynamic URL Redirects Write` on its zone, for the rulesets in §4. `validate` checks exactly that set.

The implementation PR confirms each permission group's name against the account's list, through the plan's `precondition`, before the blog PR mints anything.

### 4. The apex, for static sites

- A workload with `static_site` keeps its `ADDRESS` claim at each of its hosts. The apex renders no DNS record for them, because their custom domains create those.
- **A second derived block, `zone_rulesets`.** Its expander, `zone-rulesets`, yields up to two items per zone, rendered through `ruleset`:
  - **`<zone>:sites-root`**: phase `http_request_transform`, with one rule per static-site host:

    ```
    expression: (http.host eq "<fqdn>" and http.request.uri.path eq "/")
    action: rewrite, uri.path.value = /index.html
    ```

  - **`<zone>:sites-www`**: phase `http_request_dynamic_redirect`, with one rule per static site that serves both `@` and `www`:

    ```
    expression: (http.host eq "www.<zone>")
    action: redirect, 301, target concat("https://<zone>", http.request.uri.path), query kept
    ```

  - A zone with no static sites renders neither.
- A host's literals are known names checked by §3 of the apex ADR, so no expression is built from free text.

### 5. `src/static-site-template`

A self-contained project: a Zola theme, versioned and released like a crate.

```
src/static-site-template/
  template.yaml        version: X.Y.Z, description, features
  theme.toml
  templates/           base, header, index, post, page, changelog, posts_feed.xml, tags/
  static/assets/css/main.css
  README.md
  LICENSE              GPL-3.0-or-later
```

**The port.** It is ported from runway's theme at 1.3.2, with every runway-only detail made configuration:

- `extra.author.{name, alternate_names, page}`, for the JSON-LD and the byline;
- `extra.labels.back`, for "Back to all posts";
- `extra.look.{color, font}`, for the body classes, with runway's `gameboy` and `ibm-plex` as defaults;
- `extra.theme_color` and `extra.og_image_size`.

**Features.** `template.yaml` declares each feature, with its default:

```yaml
features:
  feed:      { default: true,  description: Atom feed of posts. }
  tags:      { default: true,  description: Tag pages. }
  changelog: { default: false, description: Changelog section and footer link. }
  math:      { default: false, description: KaTeX, pinned by version and SRI. }
  fonts:     { default: false, description: IBM Plex Sans from Bunny Fonts. }
  banner:    { default: false, description: A banner, from extra.banner. }
  commit:    { default: false, description: The build's commit in the footer. }
```

- cobot writes the effective set into the build's `[extra.features]`: the defaults, plus what the consumer lists.
- The templates branch on it.
- A consumer enables a feature by naming it, and disables a default one with `no-<feature>`.

**Release.**

- `static-site-template` is a release unit: manifest `template.yaml`, paths the project, a changelog tier, and tag `static-site-template/vX.Y.Z`.
- Its first version is `1.0.0`.
- `layout.yaml` already allows a project under `src/` named in lowercase dashes.

**Consumption.**

- cobot `git archive`s the tag `static-site-template/v<version>`'s `src/static-site-template/` into the build's `themes/static-site-template/`.
- Only released versions resolve: never `main`, and never a path.

### 6. The blog

`workloads/noisypigeon.com/blog/` holds:

- `config.yaml`, as in §2;
- `site/`, the Zola site moved from runway's `workloads/noisypigeon.com/blog/`:
  - `config.toml`, with `theme = "static-site-template"` and its `extra` keys renamed to §5's, and without `theme_version` or `base_url`;
  - `content/` and `static/`, unchanged.

Runway's `workload.yaml` and `CHANGELOG.md` are not moved: here a changelog is the bot's, and the content's own `content/changelog/` section stays.

The apex already assigns `blog` to `@` and `www`, so the blog is served at both, with `www` redirecting.

### 7. `pigeon cobot sites`

#### `build [WORKLOAD…]`

1. It copies the workload's `site/` into `.cobot/<workload path>/site/`.
2. It resolves the template from its tag.
3. It writes `base_url` and `[extra.features]` into the copied `config.toml`. `base_url` is `https://<fqdn>` of the first assigned host.
4. It runs `zola build`, with `SITE_COMMIT_SHA` set to the short commit.

It needs no secrets. Zola is pinned in `mise.toml` (0.23.6).

#### `publish --state DIR [WORKLOAD…]`

1. It builds.
2. It reads the workload's handoff from `infra/cloudflare`'s state, as `prepare` does.
3. It derives the S3 credentials: the token's id as the access key, and the SHA-256 of its value as the secret.
4. It syncs `public/` to `https://<account_id>.eu.r2.cloudflarestorage.com/<bucket>`:
   - **list** the bucket's keys and ETags with `ListObjectsV2`;
   - **put** each new or changed file, compared by MD5 against the ETag, with a `Content-Type` from its extension;
   - **put** `x` and `x/` as copies of each `x/index.html` below the root, with `text/html; charset=utf-8`;
   - **delete** each key no longer expected.
5. It runs the live checks:
   - `/` returns 200 with `<html`;
   - a page answers 200 both with and without its trailing slash;
   - a missing page returns 404;
   - `www` returns a 301 to the apex.

Credentials exist only in memory, and join the output scrubber. Output names counts and keys, never values.

#### The S3 client

- `crates/cobot/src/s3.rs` implements SigV4 for service `s3` and region `auto`, on `ureq` with rustls.
- `hmac`, `sha2` and `md-5` become workspace dependencies.
- It supports exactly `ListObjectsV2`, `PutObject` and `DeleteObject`.

#### Workflows

- **`cobot.yml`'s `apply` job** checks out with `fetch-depth: 0` and tags, so template tags resolve. After a successful apply, it runs `pigeon cobot sites publish --state .state`, with the same passphrases.
  - A sync that changes nothing uploads nothing.
  - The daily scheduled apply republishes after any token rotation.
- **A new `sites.yml`** runs `pigeon cobot sites build` on pull requests that touch `workloads/**/site/**` or `src/static-site-template/**`, with no secrets. A PR's content is therefore built only away from the passphrases.
  - It never mentions `tofu` or `cobot apply`, so `terraform-actions.yaml` holds.

### 8. Docs

- `CLAUDE.md` names `pigeon cobot sites build|publish` and the template project, citing this ADR.
- `workloads/README.md` notes that a workload enables features `infra/cloudflare` defines.

### 9. Execution

1. **This PR**, `docs(adr): static sites on Cloudflare R2, and the site template`, holds this ADR and the two amendments.
2. **The template PR**, `feat(static-site-template): port the Zola theme`, holds §5's project, its release unit and its tier. Its merge releases `static-site-template/v1.0.0`.
3. **The engine PR**, `feat(cobot): static sites on R2`, holds:
   - §1's modules and their release units;
   - §2's schema extension and expanders;
   - §3's handoff and the `account_id` output;
   - §4's apex changes;
   - §7's commands, S3 client, `mise.toml` and workflows;
   - §8's docs.

   Its plan shows `infra/cloudflare` creating `output.account_id` once merged, and nothing else.
4. **By hand, before the blog PR**, the owner adds to `provisioner_token`, at the account and on `noisypigeon.com`: `Workers R2 Storage Write`, `Transform Rules Write` and `Dynamic URL Redirects Write`.
5. **The blog PR**, `feat(workloads): the blog on R2`, holds:
   - `site-noisypigeon-com-blog` in `infra/cloudflare/config.yaml`;
   - the widened `apex-noisypigeon.com`;
   - §6's `workloads/noisypigeon.com/blog/`.

   It follows the apex's Proton PR.

### 10. Verification

- **Unit tests:**
  - each module renders and validates;
  - each bad `static_site`, unknown feature, missing host, and `ingress` beside `static_site` fails for its own reason;
  - `site-bucket`, `site-domains` and `zone-rulesets` expand as specified;
  - the apex renders no DNS record for a static site's hosts;
  - SigV4 signs AWS's published test vectors exactly;
  - against a local mock S3 server, the sync puts new and changed files, skips unchanged ones, writes the index copies, and deletes stale keys.
- **End to end** with built-in fixtures: a workload-token handoff reaches a workload unit after `infra/cloudflare` applies, and never touches disk.
- **Against the real tools:**
  - `tofu validate` passes on the rendered blog and apex against cloudflare 5.28.0;
  - `pigeon cobot sites build` builds the moved blog with Zola 0.23.6.
- **The blog PR's plan comment shows exactly:**
  - for `infra/cloudflare`, one `cloudflare_account_token` to create and `apex-noisypigeon.com`'s change;
  - for `workloads/noisypigeon.com/blog`, one `cloudflare_r2_bucket` and two `cloudflare_r2_custom_domain`s;
  - for `workloads/cloudflare/apex`, two `cloudflare_ruleset`s;
  - nothing to destroy.
- **After the blog PR merges:** `publish` uploads the site, every live check passes, the next plan shows "No changes", and #32 is closed.

## Amendments

### 2026-11-03: The apex is per zone

The workloads ADR's amendment of this date moves each zone's apex to `workloads/<zone>/apex/`. That supersedes every mention of `workloads/cloudflare/apex` here, which stays as written. The noisypigeon.com apex is `workloads/noisypigeon.com/apex/`.

Nothing else changes:

- the apex still renders `<zone>:sites-root` and `<zone>:sites-www`, with the same keys;
- §10's plan check counts those two rulesets under `workloads/noisypigeon.com/apex`.

### 2026-11-03: Two engine PRs, the token's id, and the template's defaults

**Two engine PRs.** §9's step 3 becomes two PRs, in order. Each is reviewable alone, and the first lands without touching a live unit.

- **3a**, `feat(cobot): static-site workloads on R2`, holds §1's modules, §2's schema extension and expanders, §3's handoff and outputs, and §4's apex changes. Its plan shows `infra/cloudflare` creating its new outputs, and nothing else.
- **3b**, `feat(cobot): sites build and publish`, holds §7's commands, S3 client, `mise.toml` and workflows, and §8's docs.

The blog PR follows both.

**The token's id.** `publish` signs with the site token's id as the S3 access key (§7, step 3), and `infra/cloudflare` outputs only the token values. It therefore gains a `token_ids` output, `$module.tokens[*].id`, beside `tokens` and `account_id`. `publish` reads all three from `infra/cloudflare`'s state; the `workload-token` handoff to `tofu` stays `api_token`, `zone_id` and `account_id`.

**The template's defaults.** The released `static-site-template/v1.0.0` decides its features and their defaults, which supersedes §5's table (left as written):

- on by default: `feed`, `fonts` and `commit`;
- off by default: `tags`, `changelog`, `math` and `banner`.

cobot reads them from the pinned tag's `template.yaml`, as §5 says. The theme's own fallbacks sit in `theme.toml`'s `[extra]`, where an unset feature is off, because Zola 0.23.6 rejects a missing field.
