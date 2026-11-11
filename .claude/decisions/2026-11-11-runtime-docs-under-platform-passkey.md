## The architecture site at platform.pigeon.dev/docs/, behind a passkey, and docs.pigeon.dev torn down

* Date: 2026-11-11
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `config.yaml`, `infra/pigeon/workers/control/worker.yaml`, `catalog/cloudflare/kinds/static-site/schema.json`, `catalog/pigeon/kinds/platform/schema.json`, `src/pigeon-cli/crates/backplane/src/cobot/sites.rs`, `src/pigeon-cli/crates/backplane/src/cobot/static_site.rs`, `src/site-templates/architecture/templates/index.html`, `src/pigeon-cli/crates/backplane/src/worker/mod.rs`, `src/pigeon-cli/crates/backplane/src/worker/passkeys.rs`

## Context

- **The architecture site is `sites/docs`** in `config.yaml`, a
  `cloudflare/static-site` with `hosts: [docs]`. It owns an R2 bucket, that
  bucket's custom domain on `docs.pigeon.dev`, and the zone's rewrite rules
  for that host. `MainRun`'s pipeline publishes it on merge (`sites.publish`).
  [`2026-11-05-runtime-architecture-site-through-backplane.md`](2026-11-05-runtime-architecture-site-through-backplane.md)
  §4 made it public, outside Access.
- **The platform is one hostname,** `platform.pigeon.dev`, per
  [`2026-11-10-runtime-platform-hostname-dashboard-observability.md`](2026-11-10-runtime-platform-hostname-dashboard-observability.md).
  One Access application covers it all. The Worker refuses any request
  without an Access token except `/builds/`. `/` is the dashboard's.
- **Passkeys prove the owner** on proof pages, per
  [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
  §4. A challenge is minted for a `purpose`, today `register` or
  `grant:<id>`; its relying party is `platform.pigeon.dev`.
- **cobot has no way to destroy a unit.** A unit dropped from `config.yaml`
  leaves its resources live and unmanaged.

The owner asked for:
- the site served at `platform.pigeon.dev` instead, under `/docs/`;
- a passkey asked for before any of its data is shown;
- `docs.pigeon.dev` torn down, with no redirect;
- the bucket kept for now; deleting it is a later, separate task.

## Implementation plan.

### 1. Where: `https://platform.pigeon.dev/docs/`

- Every path under `/docs/` is the site, behind the platform's Access
  application like every other path. No Access bypass is added.
- `/docs` redirects 301 to `/docs/`. `blue.platform.pigeon.dev/docs/` serves
  the same, as every page does.
- This replaces the architecture-site ADR's "Access does not cover it", and
  its 2026-11-11 amendment's "`docs.pigeon.dev` is public".

### 2. A passkey before any data

- **Only a person sees it.** A service token's request to `/docs/` is refused
  403. Agents read the same graph with `architecture_describe`.
- **Without a view cookie, every `/docs/` path answers the gate page,** 200,
  whatever the path, so it shows no file's existence. The gate page:
  - holds no site data: a title, a "Prove with a passkey" button and a
    status line;
  - loads `/passkey.js`, with `script-src 'self'` and `connect-src 'self'`;
  - asks for a challenge with purpose `view:docs`, and posts the answer to
    the path it was opened on;
  - says, without the script, that viewing needs a passkey. There is no code
    field: a code proves approvals, not viewing.
- **The challenge.** `/passkeys/challenge` accepts `view:docs` beside
  `register` and `grant:`. It is minted, kept 5 minutes and used once, as
  today.
- **The proof.** `passkeys::prove` with purpose `view:docs`, unchanged. A
  failure counts toward the same lock as codes and approvals: five in an
  hour. The post passes `page::same_origin`.
- **The view cookie,** set on a proof, then 303 back to the path:
  - `__Host-pigeon-docs`, `Path=/`, `Secure`, `HttpOnly`, `SameSite=Strict`,
    `Max-Age=43200` (12 hours);
  - its value is `<expiry>.<mac>`, the mac being HMAC-SHA256 under
    `SESSION_KEY` of `docs-view|<Access subject>|<expiry>`. It holds no
    secret, works for one Access identity, and needs no new secret;
  - an expired, malformed or other identity's cookie answers the gate.
- **No passkey registered yet** answers the gate with a link to `/passkeys`.

### 3. Serving the site

- **The Worker reads the site's bucket** through an R2 binding, `DOCS`, and
  only reads it. The bucket's name is never in the repository:
  - the `pigeon/platform` kind gains `sites: { docs: sites/docs }`;
  - `worker.yaml` binds `DOCS: "$deployment.sites[docs].bucket"`, resolved
    from `DEPLOY_OUTPUTS`, which `pigeon cobot workers outputs` fills from
    the site unit's state.
- **After the gate,** `/docs/<p>` reads `<p>`; a path ending in `/` reads
  `<p>index.html`. A missing key answers the site's `404.html` with 404.
- **Headers:** content type by extension; `cache-control: private, no-store`;
  and a page policy: `default-src 'none'`, `script-src 'self'`,
  `style-src 'self' https://fonts.bunny.net`, `font-src https://fonts.bunny.net`,
  `img-src 'self' data:`, `connect-src 'self'`, `frame-ancestors 'none'`.
- **`/healthz/smoke`** gains `docs`: whether `DOCS` holds `index.html`.

### 4. Building the site at a path

- **The `cloudflare/static-site` kind gains `served_by`:**
  `{ unit: platform/blue, path: /docs }`. With it, `hosts` may be empty, and
  the kind renders no custom domain and no rewrite rules for the site.
- **`base_url`** is `https://<the unit's domains[0]><path>`.
- **The architecture template links relative to `base_url`:**
  `manifest.json`, `assets/css/main.css` and `assets/js/rail.js`. A test
  builds it at a path and finds no root-relative link.
- **The live checks.** A `served_by` site is behind the gate, so
  `sites.publish` checks its uploaded digests, as today, and no URL. The next
  deploy's `/healthz/smoke` shows `docs`.

### 5. Tearing down `docs.pigeon.dev`

- **`sites/docs` drops `hosts: [docs]`,** keeping `served_by`.
- **Its `infra.plan` destroys** the bucket's custom domain on
  `docs.pigeon.dev`, which removes its DNS record, and that host's rewrite
  rules. Nothing else. The apply is `owner` tier (approvals ADR §2).
- **No redirect.** `docs.pigeon.dev` stops resolving.
- **The bucket stays,** read by the Worker, until the owner's clean-up task.
- `describe`'s `network` gains the `/docs/` route and a `store` link from the
  Worker to R2, each with its evidence row; the Access domains are unchanged.

### Execution

Each step is its own change.

1. **This ADR.**
2. **`feat(cobot)`:** `served_by` in the static-site kind and `sites.rs`'s
   `base_url` and checks; the template's relative links, with the test.
3. **`feat(backplane)`:** `/docs/`: the gate, `view:docs`, the cookie, the
   serving and its headers, and `docs` in the smoke check, with tests for
   each refusal in §2.
4. **`feat(infra)`:** the kind's `sites`, `worker.yaml`'s `DOCS`, and
   `sites/docs` gains `served_by`, keeping `hosts`. Both URLs then serve; the
   owner registers a passkey on `platform.pigeon.dev` if none is, and opens
   `/docs/`.
5. **`chore(infra)`:** `sites/docs` drops `hosts`; `infra.plan`, then the
   owner's `infra.apply`.
6. **`feat(backplane)`:** `describe`'s route and link rows.

### Verification

- `/docs/`, `/docs/assets/css/main.css` and `/docs/nope` each answer the
  gate, with no site byte, without a cookie.
- A passkey proof sets the cookie and shows the diagram, styled, with
  `rail.js` routing its line.
- A replayed proof, a `grant:` challenge's answer, an expired cookie and
  another identity's cookie are each refused. Five failures lock the hour.
- A service token's `/docs/` answers 403.
- After step 5, `docs.pigeon.dev` doesn't resolve, the plan destroyed only
  its domain and rules, and the next merge publishes the site, which
  `/healthz/smoke` reports.

## Amendments

### 2026-11-11: cobot's modules move into backplane, and this ADR's paths with them

Reviewed against
[`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md),
whose step 6b (!198) moves the `cobot` crate into `backplane` as
`backplane::cobot`.

- **The two agree.** That move changes no behaviour, and nothing here depends
  on cobot being its own crate.
- **Paths:** `src/pigeon-cli/crates/cobot/src/sites.rs` and `static_site.rs`
  become `src/pigeon-cli/crates/backplane/src/cobot/sites.rs` and
  `static_site.rs`. !198 rewrites this ADR's `Governs` line to match, as it
  does every ADR's.
- **Step 2's `feat(cobot)`** edits those files at their new paths once !198
  merges, and its tests are backplane's `cobot_sites` and `cobot_manifest`.
  `pigeon cobot sites …` is unchanged.
