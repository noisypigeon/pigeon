## Zone DNS vendors: presets placed at the apex or any subdomain, Proton alias domains first on platform.pigeon.dev

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `catalog/cloudflare/presets/**`, `catalog/cloudflare/kinds/zone/schema.json`, `src/pigeon-cli/crates/backplane/src/cobot/apex.rs`, `src/pigeon-cli/crates/backplane/tests/cobot_apex.rs`

## Context

The owner wants one reusable way to turn on a vendor's DNS in a zone, at the
apex or at any subdomain. More vendors will follow. The first is Proton's alias
domains, and the first test puts them at `platform.pigeon.dev`.

### What exists

- **Presets.**
  [`2026-11-02-runtime-workloads-zone-apex.md`](2026-11-02-runtime-workloads-zone-apex.md)
  §4 to §6 made a vendor a data file of inputs, records and claims. It lives in
  `catalog/cloudflare/presets/<service>.yaml`, per
  [`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md).
  - A zone turns one on with `services.<service>`.
  - Records are keyed `<service>-<record>`.
  - `records.<key>` overrides a record field by field, or drops it with
    `false`.
  - Claims catch two owners of one name's `MX`, `SPF`, `DMARC` or address.
- **Zone units.**
  [`2026-11-05-runtime-instance-config-kind-units.md`](2026-11-05-runtime-instance-config-kind-units.md)
  §2 made a zone a `cloudflare/zone` unit in an instance's `config.yaml`.
  - The `zone-records` expander (`cobot/apex.rs`) expands its `services` and
    `records`.
  - It checks claims against the manifest's sites. It doesn't see a
    `pigeon/platform` unit's `domains`.
  - Inputs marked `published` are plain values.
- **`pigeon.dev`** is `zones/pigeon.dev` in pigeon's `config.yaml`, with
  `services: {}` and `records: {}`.
  - The owner applies it with `platform.plan` and `platform.apply`, per
    [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md).
  - Its token already holds `dns.write` on the zone.
- **`platform.pigeon.dev`** is a Workers custom domain of `platform/blue`
  (`kind: pigeon/platform`), behind Access. Cloudflare holds its DNS record,
  outside the zone unit's state.
- **The one preset, `proton`,** is Proton's alias-domain record set:
  - `MX` at `mx1` and `mx2.alias.proton.me`;
  - SPF `include:alias.proton.me`;
  - three `dkim*._domainkey` `CNAME`s into `alias.proton.me`;
  - the `protonmail-verification` and `pm-verification` `TXT` records;
  - DMARC.

  `noisypigeon.com` uses it at its apex.

### The gaps

1. **A preset only knows the apex.** Its names are relative to the zone, so
   `@ MX` is always the zone's own MX. Alias domains at `platform.pigeon.dev`
   can only be written out as raw `records`, and then they make no claims.
2. **One service per vendor per zone.** `services` is keyed by the preset's
   name, so a vendor can't sit at both `@` and `platform`.
3. **A vendor can't say where it works,** so nothing stops placing one where it
   doesn't.
4. **A zone doesn't see the platform's domains,** so it can't check records
   against them.

### The decisions

- **A service is a vendor's preset placed at a name.** `services.<name>` gains
  two reserved keys:
  - `preset`, which defaults to `<name>`;
  - `at`, a name relative to the zone, which defaults to `@`.

  The preset's names are relative to `at`.
- **Today's configs render unchanged.** With no `preset` or `at`, keys, names
  and contents come out exactly as they do now, so `noisypigeon.com` plans no
  change.
- **A preset declares its `placements`** (`apex`, `subdomain`, or both), and a
  `docs` link to where the vendor shows its per-domain values.
- **`proton` allows both placements,** with its records and keys unchanged.
- **A zone claims an address for each `pigeon/platform` domain in it,** so its
  checks see the Worker's hosts.
- **`platform.pigeon.dev` is the first test,** with `proton` at `platform`.

Out of scope, each for a later amendment:

- record types beyond `A`, `AAAA`, `CNAME`, `MX` and `TXT`. `SRV` and `CAA`
  will be the first a vendor needs.
- anything a vendor needs outside DNS, such as calls to its API;
- DNS providers other than Cloudflare. Presets are provider-neutral data, and
  move to a shared catalog when a second DNS provider arrives;
- importing records that are already in Cloudflare outside state;
- a Proton Mail mailbox preset (`mail.protonmail.ch`), when one is wanted.

## Implementation plan.

### 1. A service's config

In a `cloudflare/zone` unit:

```yaml
zones/pigeon.dev:
  kind: cloudflare/zone
  services:
    platform-aliases:            # preset proton, at platform.pigeon.dev
      preset: proton
      at: platform
      verification: { protonmail: <code>, pm: <code> }
      dmarc: default
  records: {}
```

- **The key** is the service's name, in kebab-case as now. It owns every record
  and claim the service makes, and prefixes its record keys.
- **`preset`**, optional: a file in `catalog/cloudflare/presets/`. It defaults
  to the key, which is how `noisypigeon.com`'s `services.proton` reads today.
- **`at`**, optional: where the service sits, relative to the zone. It is `@`,
  or a name matching the schema's `$defs/name` with no wildcard and no label
  starting `_`. It defaults to `@`.
- **The rest** are the preset's inputs, checked as now.
- **A preset may not declare an input named `preset` or `at`.**
  `check_preset` rejects one.

### 2. Placing a preset's names

Each name in a preset's records and claims is rebased onto `at`:

| `at` | preset name | rendered name |
| --- | --- | --- |
| `@` | `@` | `@` |
| `@` | `_dmarc` | `_dmarc` |
| `platform` | `@` | `platform` |
| `platform` | `dkim._domainkey` | `dkim._domainkey.platform` |
| `platform` | `_dmarc` | `_dmarc.platform` |

- Rebasing happens before claims and the record rules, so both see final names.
- A rebased name over 253 characters with the zone, or with a label over 63, is
  rejected, naming the service.
- **The CNAME rule is unchanged.** Cloudflare flattens a `CNAME` only at the
  zone's `@`. So a service at a name that a site serves through a `CNAME` is
  rejected, for example `platform holds a CNAME (host-…), which shares its
  name with no other record: platform-aliases-mx1`.
- **Another zone unit wins.** Suppose the manifest has
  `zones/platform.pigeon.dev`. Then `at: platform`, or any name under it, in
  `zones/pigeon.dev` is rejected with `platform.pigeon.dev is its own zone
  unit; place the service there at @`.

### 3. Record keys

- A rendered record's key stays `<service>-<record>`, for example
  `platform-aliases-mx1`. For a service with no `preset`, that is today's key.
- Two services whose keys collide are rejected, naming both. For example,
  `proton` with record `mail-mx` and `proton-mail` with record `mx` both give
  `proton-mail-mx`.
- `records.<key>` overrides and drops records as now. A key with an enabled
  service's prefix is still rejected if it matches no rendered record of any
  such service.
- `published_values` finds a service's preset through `preset`, not through its
  key.

### 4. The platform's domains

`Site` gives the zone the `domains` of every `pigeon/platform` unit in the
manifest that fall in it. Each becomes an `ADDRESS` claim owned by
`platform <unit>`, which renders no record, the same way a site claims a host
that it doesn't render.

- `MX` and `TXT` at the same name don't conflict with it. Records at a name
  under it, such as `dkim._domainkey.platform`, don't either.
- An `ADDRESS` record at that name from a preset or from `records` is rejected,
  for example `platform ADDRESS is claimed by platform platform/blue and
  records.x`.

### 5. The preset file

The preset file gains two top-level fields. Unknown keys are still rejected.

- **`placements`**: a non-empty subset of `[apex, subdomain]`, defaulting to
  both. A service whose `at` the preset doesn't allow is rejected, for example
  `services.x: preset y allows only apex`.
- **`docs`**: an `https://` link to the vendor's page for the per-domain values.
  It is required on every new preset, and the rejection for a missing required
  input shows it.

In `proton.yaml`:

- `placements: [apex, subdomain]`;
- `docs:` Proton's page on adding a custom domain to Proton Pass aliases;
- the description becomes "Proton's alias domains (`alias.proton.me`) at a zone
  apex or subdomain.";
- the comment cites this ADR beside the zone-apex one.

Its records, claims and keys don't change.

### 6. The kind schema

In `catalog/cloudflare/kinds/zone/schema.json`, the description of `services`
says each entry is a placed preset with optional `preset` and `at`. Its shape
(`additionalProperties: object`) doesn't change, since cobot checks a service's
entries against its preset. The `infra-cloudflare-schema` release picks up the
preset and schema changes, per `.config/releases.yaml`.

### 7. Adding a vendor later

Adding a vendor is one change. It adds a preset file with `description`,
`docs`, `placements`, `inputs`, `claims` and `records`, and a test that renders
it at `@` and at a subdomain. A vendor that needs a new record type or claim
type needs an amendment here first.

### Execution

1. **This ADR,** as its own change.
2. **`feat(backplane)`: placed services.** It covers `preset`, `at`, rebasing,
   `placements`, `docs`, key collisions, the other-zone-unit check,
   `published_values` and the platform-domain claims (§4), with the tests in
   §Verification. It merges before any config uses the new fields.
3. **`feat(catalog)`:** `proton.yaml`'s `placements`, `docs`, description and
   comment, and the kind schema's description.
4. **The owner, by hand:**
   - adds `platform.pigeon.dev` as a custom alias domain in Proton Pass;
   - notes its verification codes;
   - checks in Cloudflare that `platform.pigeon.dev` has no `MX`, SPF `TXT` or
     `_dmarc.platform` record outside state, and that the dashboard accepts an
     `MX` beside the Worker's custom-domain record. If it doesn't, the change
     stops here and this ADR gets an amendment.
5. **`feat(config)`: Proton alias domains on `platform.pigeon.dev`.** It adds
   `zones/pigeon.dev.services.platform-aliases` as in §1. Its `platform.plan`
   creates 9 records and changes nothing else:
   - 2 `MX`;
   - 3 DKIM `CNAME`s;
   - the SPF `TXT`;
   - 2 verification `TXT`s;
   - the DMARC `TXT`.

   The owner applies it.
6. **The live check:**
   - `dig` shows the records at `platform.pigeon.dev`,
     `dkim{,02,03}._domainkey.platform.pigeon.dev` and
     `_dmarc.platform.pigeon.dev`;
   - the Worker still answers at `platform.pigeon.dev`;
   - Proton verifies the domain;
   - a message sent to a new alias there arrives.

### Verification

- **No change for today's zones.** `noisypigeon.com`'s zone unit renders the
  same record keys, names and contents before and after step 2.
- **Rebasing.** `proton` at `@` and at `platform` renders the names in the §2
  table. Claims at `platform` don't conflict with claims at `@`.
- **Two placements of one vendor** in one zone render side by side, each under
  its own keys.
- **Platform domains.** `proton` at `platform` passes beside `platform/blue`'s
  `platform.pigeon.dev`. A `records` `CNAME` at `platform` is rejected.
- **Rejections,** each naming its source:
  - `at` with a wildcard or a `_` label;
  - a name over the length limits;
  - a placement the preset doesn't allow;
  - a preset input named `at` or `preset`;
  - colliding record keys;
  - `at` inside another zone unit of the manifest;
  - a service at a name that a site serves through a `CNAME`.
- **The live checks** in execution step 6.

## Amendments

### 2026-11-13: The verification codes are asked at the config change's plan approval

[`2026-11-13-security-approvals-ask-owner-inputs.md`](2026-11-13-security-approvals-ask-owner-inputs.md)
lets a change's plan approval ask the owner for inputs, on the approval page
and in the app. Execution steps 4 and 5 change accordingly.

- **Step 4:** the owner still adds `platform.pigeon.dev` in Proton Pass and
  checks Cloudflare, but sends nothing in chat.
- **Step 5:** the agent proposes the config change with two asks:
  - `protonmail_verification`, labelled "protonmail-verification code";
  - `pm_verification`, labelled "pm-verification code";

  each with help naming Proton Pass's Settings → Aliases →
  `platform.pigeon.dev` → Info.
- **The owner authorizes the plan in the app,** typing both codes there.
- **The agent reads them from `change_status`'s `answers`,** writes them into
  `zones/pigeon.dev.services.platform-aliases.verification`, and pushes.
- **Steps 2 to 4 of the asks ADR come first.** Until they've landed, step 5
  waits.
