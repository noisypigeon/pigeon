## Workloads by zone, each with an apex

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`workloads/` has been empty since `.claude/decisions/2026-11-02-development-repository-area-layout.md` reserved it. `infra/cloudflare` now mints rotating, scoped tokens (`.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md`), and that ADR left "handing minted tokens to workloads" and "DNS records" to later. This ADR takes up both and sets the precedent for everything under `workloads/`.

The decisions:

- **Workloads live by zone.** `workloads/<zone>/` is named for a zone listed in `infra/cloudflare/config.yaml`'s `zones`, e.g. `workloads/noisypigeon.com/`. Each directory inside it is one workload, and its name is the workload's name.
- **Every zone has an apex.** `workloads/<zone>/apex/` is a required config unit:
  - it holds the zone's global configuration and its DNS records;
  - it assigns which workload serves the apex: `noisypigeon.com/apex` assigns `blog`, so `workloads/noisypigeon.com/blog` loads at `@` and `www`;
  - every other workload becomes `<workload>.<zone>` by default, and the apex may override a workload's hosts.
- **One schema for every apex.** The apex's `schema.json`, its single-resource modules and its service presets live once, in `workloads/.apex/`. They are versioned and released exactly as `infra/cloudflare`'s are.
- **Services are data.** A service such as Proton Mail is a preset file that declares its inputs, its records and what it claims. Adding a provider is adding one file.
- **Workloads may hold secrets, under their own passphrase.** `COBOT_WORKLOAD_SECRETS_PASSPHRASE` is a third passphrase. Break-glass keys stay in `infra/` and are rejected under `workloads/`.
- **The apex's Cloudflare credentials are minted, never stored.** `infra/cloudflare` mints one zone-scoped DNS token per zone. cobot reads the live generation from `infra/cloudflare`'s encrypted state and hands it to that zone's apex at plan and apply time, only through the `tofu` process's environment.

This is modelled on runway's DNS composition, and fixes its weakness. In runway, `zone.yaml` enables `services.proton`, a fixed table in `.config/root.hcl` expands it into records keyed `<service>-<record>`, a `dnsRecords` map overrides an implied record field by field, and guards stop two mail services both claiming MX and SPF at `@`. Adding a provider there means editing three places in sync. Here, the table, the claims and the inputs are one preset file. Nothing of runway's is copied.

This amends one part of the cloudflare ADR, by an amendment appended to it: §4's rule that workloads never hold or reference secrets.

Out of scope, each for a later ADR or amendment:

- the blog workload itself, its content and its release pipeline: [#32](https://github.com/noisypigeon/pigeon/issues/32);
- any workload schema beyond the `ingress` contract in §3;
- Cloudflare zone settings (SSL mode, HTTPS rewrites and the like), which later join the apex schema;
- record types beyond `A`, `AAAA`, `CNAME`, `MX` and `TXT`;
- presets beyond `proton`;
- tokens for workloads other than the apex's;
- importing records that already exist in Cloudflare.

## Implementation plan.

### 1. Layout

```
workloads/
  README.md
  .apex/                     shared by every apex; never a unit
    schema.json              JSON Schema plus x-cobot; versioned
    modules/dns-record/      one resource; individually versioned
    presets/<service>.yaml   one file per service
  <zone>/                    a zone in infra/cloudflare's zones
    apex/                    required config unit
      config.yaml
      secrets.config.yaml    optional
    <workload>/              one unit per workload
```

`pigeon cobot validate` rejects:

- a directory directly under `workloads/` that is neither `.apex` nor a zone in `infra/cloudflare/config.yaml`'s `zones`;
- a zone directory without `apex/config.yaml`;
- an apex that holds its own `schema.json` or `modules/`, or that is a `unit.yaml` unit;
- a file directly in a zone directory;
- a workload named `apex`, or not lowercase words joined by dashes.

An apex is a config unit whose `schema.json` and `modules/` are `workloads/.apex/`'s. `validate` also scans `workloads/.apex/` with the unit rules: one resource per module, allowlisted providers, no provisioners, no functions that read the runner.

`.config/policies/layout.yaml`'s `workloads` rule tightens to:

```yaml
  - entries:
      path: workloads
      dirs: ['^\.apex$', '^[a-z0-9-]+(\.[a-z0-9-]+)+$']
      files: ['^README\.md$']
      require: [README.md, .apex]
```

`workloads/README.md` says what the area holds and links to this ADR.

### 2. The apex config

`workloads/noisypigeon.com/apex/config.yaml`, in full:

```yaml
# The noisypigeon.com apex; see .claude/decisions/2026-11-02-runtime-workloads-zone-apex.md.
apex: blog
services:
  proton:
    verification:
      protonmail: { secret: protonmail_verification }
      pm: { secret: pm_verification }
    dmarc: default
records: {}
```

- **`apex`**, optional: the workload served at `@` and `www`.
- **`hosts`**, optional: a map from a workload to its names, relative to the zone, replacing its default. The apex workload's default is `[@, www]`; every other workload's is `[<workload>]`.
- **`services`**: a map from a preset's name to its inputs (§4).
- **`records`**: a map of explicit records, keyed in kebab-case (§5).

Names are always relative to the zone: `@`, `www`, `dkim._domainkey`. A name ending in `.<zone>` is rejected. cobot renders each as its fully qualified name, which is what the provider stores.

### 3. Hosts

**Assignment.** Each workload unit under `workloads/<zone>/`, and the `apex` workload whether or not it exists, gets its hosts from §2.

**Validation:**

- every workload that `hosts` names must exist as a unit under `workloads/<zone>/`;
- the `apex` workload need not exist. Until it does, its hosts are still claimed, and render no record. So the precedent lands before `blog` does;
- no host is claimed twice (§6).

**Records.** A workload says how it is reached with a top-level `ingress` in its own `config.yaml`:

```yaml
ingress: { type: CNAME, content: <target>, proxied: true }
```

- `type` is `A`, `AAAA` or `CNAME`; `proxied` defaults to `true`.
- It is checked against the apex schema's `$defs/ingress`. A workload's own schema must allow it.
- For each host of a workload with an `ingress`, the apex renders one record, keyed `host-<workload>-<label>`, where `<label>` is the host with `@` as `apex` and dots as dashes: `host-blog-apex`, `host-blog-www`.
- A workload without `ingress`, or a unit-schema-v0 workload, keeps its claim and renders nothing.

### 4. Presets

A preset is `workloads/.apex/presets/<service>.yaml`. Unknown keys are rejected.

```yaml
# Proton Mail; see .claude/decisions/2026-11-02-runtime-workloads-zone-apex.md.
description: Proton Mail at the zone apex.
inputs:
  verification.protonmail: { type: string, secret: true, optional: true }
  verification.pm: { type: string, secret: true, optional: true }
  dmarc:
    type: string
    optional: true
    default: "v=DMARC1; p=quarantine; pct=100; adkim=s; aspf=s"
claims:
  - { name: "@", type: MX }
  - { name: "@", type: SPF }
  - { name: dkim._domainkey, type: ADDRESS }
  - { name: dkim02._domainkey, type: ADDRESS }
  - { name: dkim03._domainkey, type: ADDRESS }
  - { name: _dmarc, type: DMARC, when: dmarc }
records:
  mx1: { name: "@", type: MX, content: mx1.alias.proton.me, priority: 10 }
  mx2: { name: "@", type: MX, content: mx2.alias.proton.me, priority: 20 }
  dkim: { name: dkim._domainkey, type: CNAME, content: dkim._domainkey.alias.proton.me }
  dkim02: { name: dkim02._domainkey, type: CNAME, content: dkim02._domainkey.alias.proton.me }
  dkim03: { name: dkim03._domainkey, type: CNAME, content: dkim03._domainkey.alias.proton.me }
  spf: { name: "@", type: TXT, content: "v=spf1 include:alias.proton.me ~all" }
  verification-protonmail: { name: "@", type: TXT, content: "protonmail-verification=${verification.protonmail}" }
  verification-pm: { name: "@", type: TXT, content: "pm-verification=${verification.pm}" }
  dmarc: { name: _dmarc, type: TXT, content: "${dmarc}" }
```

**Inputs.** Each key is a dotted path into the service's config. Each spec has:

- `type`: `string`, `integer` or `boolean`;
- `secret: true`: the value may be `{ secret: <key> }`, a key in the apex's `secrets.config.yaml`;
- `default`: used when the input is absent, or given as the string `default`;
- `optional: true`: the input may be `false`, or absent with no `default`, and is then unset.

A required input that is unset, an unknown path, a value of the wrong type, and `{ secret: … }` on an input without `secret: true` are rejected.

**Records.** Each is `{ name, type, content, ttl?, priority?, proxied? }`, under the record rules of §5.

- `${<input>}` in `content` is replaced by the input's value. A literal is escaped, so it is never evaluated. A secret becomes `${var.secrets["<key>"]}`, so its value is never written to disk.
- A record that references an unset input is not rendered.
- Its key is `<service>-<record>`: `proton-mx1`, `proton-spf`, and so on.
- A `${…}` naming no input is rejected.

**Claims.** Each is `{ name, type, when? }`. `type` is `ADDRESS` (any of `A`, `AAAA` and `CNAME`), `MX`, `SPF` or `DMARC`. A claim with `when` holds only while that input is set.

### 5. Records

`records` maps a kebab-case key to a record, or to `false`.

- **A new key** is a full record: `name`, `type` and `content` are required.
- **A key equal to a preset or host record's key** (`proton-spf`, `host-blog-www`) overrides that record field by field. `false` drops it.
- A key starting with an enabled service's `<service>-`, or with `host-`, that names no such record is rejected, so a typo never adds a stray record.
- `content` may be `{ secret: <key> }`.

The record rules, applied after merging:

- `type` is `A`, `AAAA`, `CNAME`, `MX` or `TXT`;
- `ttl` is `1` (automatic, the default) or 60 to 86400;
- `priority` is required for `MX`, and rejected otherwise;
- `proxied` defaults to `false`, and is allowed only for `A`, `AAAA` and `CNAME`.

### 6. Claims and conflicts

Claims are taken after every override has merged, so they describe the records that will actually render.

**Who claims what:**

- an enabled preset claims what it declares, whether or not its records survive overrides;
- a workload reserves `ADDRESS` at each of its hosts, whether or not it renders;
- every rendered record claims by its final name, type and content: `ADDRESS` for `A`, `AAAA` or `CNAME`; `MX`; `SPF` for a `TXT` starting `v=spf1`; `DMARC` for a `TXT` starting `v=DMARC1`. Any other `TXT` claims nothing, since many may share a name.

A rendered record's owner is where it came from: its preset, its workload, or `records.<key>` for a new record. An override keeps the owner of the record it changes, so `records.host-blog-www: { name: shop }` claims `shop` for `workload blog`, and conflicts with `workload shop`.

**Conflicts, each rejected naming both sides:**

- two owners claiming the same name and type: `@ MX is claimed by proton and records.backup-mx`;
- a `CNAME` at any name but `@` beside any other rendered record at that name, whoever owns them: `mail holds a CNAME (host-mail-mail), which shares its name with no other record: backup-mx`. Cloudflare flattens a `CNAME` at `@`, so there it may sit beside `MX` and `TXT`.

### 7. The apex schema

`workloads/.apex/schema.json`, in full:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "workloads/<zone>/apex",
  "description": "A zone's apex: its global config and DNS; see .claude/decisions/2026-11-02-runtime-workloads-zone-apex.md.",
  "type": "object",
  "additionalProperties": false,
  "required": ["services", "records"],
  "x-cobot": {
    "version": "0.1.0",
    "secrets": "referenced",
    "handoff": "zone-token",
    "providers": {
      "cloudflare": { "api_token": "$handoff.api_token" }
    },
    "derived": {
      "dns_records": {
        "expand": "zone-records",
        "module": "dns-record",
        "inputs": {
          "zone_id": "$handoff.zone_id",
          "name": "$item.name",
          "type": "$item.type",
          "content": "$item.content",
          "ttl": "$item.ttl",
          "priority": "$item.priority",
          "proxied": "$item.proxied"
        }
      }
    }
  },
  "properties": {
    "apex": {
      "description": "The workload served at @ and www.",
      "$ref": "#/$defs/workload"
    },
    "hosts": {
      "description": "A workload's names, relative to the zone, replacing its default.",
      "type": "object",
      "propertyNames": { "$ref": "#/$defs/workload" },
      "additionalProperties": {
        "type": "array",
        "minItems": 1,
        "uniqueItems": true,
        "items": { "$ref": "#/$defs/name" }
      }
    },
    "services": {
      "description": "Presets by name, each workloads/.apex/presets/<name>.yaml; cobot checks their inputs.",
      "type": "object",
      "propertyNames": { "$ref": "#/$defs/key" },
      "additionalProperties": { "type": "object" }
    },
    "records": {
      "description": "Explicit records by key; a preset or host record's key overrides it field by field, and false drops it.",
      "type": "object",
      "propertyNames": { "$ref": "#/$defs/key" },
      "additionalProperties": {
        "oneOf": [{ "const": false }, { "$ref": "#/$defs/record" }]
      }
    }
  },
  "$defs": {
    "key": { "type": "string", "pattern": "^[a-z0-9]+(-[a-z0-9]+)*$" },
    "workload": {
      "type": "string",
      "pattern": "^[a-z0-9]+(-[a-z0-9]+)*$",
      "not": { "const": "apex" }
    },
    "name": {
      "type": "string",
      "pattern": "^(@|(\\*\\.)?[a-z0-9_]([a-z0-9_-]*[a-z0-9_])?(\\.[a-z0-9_]([a-z0-9_-]*[a-z0-9_])?)*)$"
    },
    "secret": {
      "type": "object",
      "additionalProperties": false,
      "required": ["secret"],
      "properties": { "secret": { "type": "string", "pattern": "^[a-z0-9_]+$" } }
    },
    "record": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "name": { "$ref": "#/$defs/name" },
        "type": { "enum": ["A", "AAAA", "CNAME", "MX", "TXT"] },
        "content": {
          "oneOf": [{ "type": "string", "minLength": 1 }, { "$ref": "#/$defs/secret" }]
        },
        "ttl": {
          "type": "integer",
          "anyOf": [{ "const": 1 }, { "minimum": 60, "maximum": 86400 }]
        },
        "priority": { "type": "integer", "minimum": 0, "maximum": 65535 },
        "proxied": { "type": "boolean" }
      }
    },
    "ingress": {
      "type": "object",
      "additionalProperties": false,
      "required": ["type", "content"],
      "properties": {
        "type": { "enum": ["A", "AAAA", "CNAME"] },
        "content": { "type": "string", "minLength": 1 },
        "proxied": { "type": "boolean" }
      }
    }
  }
}
```

**New `x-cobot` forms**, all checked by `validate`:

- **`secrets: "referenced"`**: the unit's secret keys are exactly the `{ secret: <key> }` values its `config.yaml` holds. A list, as before, still fixes them.
- **`handoff: "zone-token"`**: allowed only on `workloads/<zone>/apex`. It provides `$handoff.api_token` and `$handoff.zone_id` (§9), rendered as `${var.handoff["<key>"]}` with `variable "handoff" { type = map(string), sensitive = true }`. No other handoff kind exists, and the token's name is not configurable.
- **`derived`**: blocks whose `for_each` comes from a built-in expander rather than a config list. The only expander is `zone-records` (§3 to §6). Its items are `{ name, type, content, ttl, priority, proxied }`, keyed by record key, with names fully qualified and `priority` null where unset. `$item.<field>` works as for list blocks.

### 8. The `dns-record` module

`workloads/.apex/modules/dns-record/`, in the unified module shape:

| Resource | Inputs | Outputs |
| --- | --- | --- |
| `cloudflare_dns_record` | `zone_id`, `name`, `type`, `content`, `ttl` (default `1`), `priority` (default `null`), `proxied` (default `false`) | `id`, `name` |

It pins `cloudflare/cloudflare` exactly as the allowlist does.

### 9. The apex token, handed off

**Minting.** `infra/cloudflare/config.yaml`'s `tokens` gains one entry per zone with an apex, through the existing rotating generations:

```yaml
tokens:
  - name: apex-noisypigeon.com
    capabilities:
      - { permission: DNS Write, zones: [noisypigeon.com] }
```

- `validate` rejects an apex whose zone has no such token, and an `apex-<zone>` token with any other capability.
- `infra/cloudflare/schema.json`'s token name pattern admits dots: `^[a-z0-9]+([.-][a-z0-9]+)*$`.
- `infra/cloudflare/schema.json` gains `x-cobot.outputs`, a new form: root outputs, each a `$module` expression, always rendered `sensitive`:

  ```json
  "outputs": {
    "zone_ids": "$module.zones[*].id",
    "tokens": "$module.tokens[*].value"
  }
  ```

- A plan counts output changes as changes, and lists them as `create output.tokens`, so the first apply records them in state even with no resource to change.

**Handing it off.** Before planning an apex, cobot:

1. initializes `infra/cloudflare`'s work directory and reads its state with `tofu show -json`, as rotation already does;
2. takes `zone_ids[<zone>]`, and from `tokens` the live generation of `apex-<zone>`: the instance with the latest `not_before` among those with `not_before ≤ today < expires_on`;
3. passes both only as `TF_VAR_handoff`, in the `tofu` process's environment, and adds both values to the output scrubber. Nothing is written to disk, and everything else read from that state is dropped.

**Pending credentials.** When the state holds no live generation, or no zone id, yet:

- a **plan** uses inert placeholders and still renders every record, and its summary says: "Credentials pending: planned with placeholders until infra/cloudflare mints `apex-<zone>`." That is how the first apex PR shows its records before the token exists. A plan of new records never calls the API, which was checked against cloudflare 5.28.0;
- an **apply** refuses that unit, with exit `2`.

**Ordering.** `select` orders every `infra/` unit before every `workloads/` unit, so in one apply run `infra/cloudflare` mints the token before the apex plans. If `infra/cloudflare`'s apply fails, the run stops before any apex.

### 10. Workload secrets

- A `workloads/` unit may hold `secrets.config.yaml`, in the same `ENC[cobot:v1:…]` format, encrypted with `COBOT_WORKLOAD_SECRETS_PASSPHRASE`, never `COBOT_SECRETS_PASSPHRASE`.
- `secrets set`, `secrets check`, and a `plan` or `apply` of a workload unit with secrets, refuse (exit `2`) unless it is set, with the 16-character minimum.
- Any of them also refuses if the two secrets passphrases are equal.
- `validate` rejects, under `workloads/`, any secret key that an `infra/` unit declares (`account_id`, `provisioner_token`), and `provisioner_token` by name. `$secret` expressions are allowed under `workloads/` only for declared keys.
- `cobot.yml` sets `COBOT_WORKLOAD_SECRETS_PASSPHRASE` on the `plan` and `apply` steps, beside the other two.
- `.config/policies/terraform.yaml`'s secrets `find` rule allows `secrets.config.yaml` under `infra/` and `workloads/`, and its description says so.

A break-glass key still never reaches a workload. A workload gets only its own secrets and the tokens minted for it.

### 11. Releases

`.config/releases.yaml` gains, exactly as `infra/cloudflare`'s:

- `workloads-apex-dns-record`: manifest `workloads/.apex/modules/dns-record/module.yaml`, paths its own directory;
- `workloads-apex-schema`: manifest `workloads/.apex/schema.json`, paths `schema.json`, `modules/` and `presets/`, so a preset change bumps the schema too.

Each gets a changelog tier beside its files, written by the bot, and `release-changelog.yaml` lists both.

### 12. Docs

`CLAUDE.md` gains:

- a pointer to this ADR;
- `COBOT_WORKLOAD_SECRETS_PASSPHRASE`, beside the secrets commands;
- the rule that only `infra/` keys are break-glass, and a workload gets only its own secrets and minted tokens.

### 13. Execution

1. **PR A**, `docs(adr): workloads by zone, each with an apex`, holds this ADR and the cloudflare ADR's amendment.
2. **PR B**, `feat(cobot): zone apexes, workload secrets and token handoff`, holds §1 and §3 to §12, except the apex unit and the `apex-noisypigeon.com` token:
   - the engine in `cobot`: workload secrets, `referenced` secrets, `derived` and `zone-records`, presets, claims, `handoff`, `outputs`, output changes, ordering, and the layout checks;
   - `workloads/.apex/` with the `dns-record` module and the `proton` preset;
   - `infra/cloudflare/schema.json`'s pattern and outputs;
   - the policies, releases, `cobot.yml` and docs.

   Its tests use fixtures built only from built-in resources. Its plan shows `infra/cloudflare` creating `output.zone_ids` and `output.tokens`, and nothing else. Its merge releases `cobot`, the apex schema and module, and the cloudflare schema.
3. **By hand, before PR C**, the owner:
   - generates `COBOT_WORKLOAD_SECRETS_PASSPHRASE`, distinct from the other two, adds it as an Actions secret, and keeps a copy in a password manager;
   - confirms `provisioner_token` can grant `DNS Write` on `noisypigeon.com`;
   - confirms `noisypigeon.com` holds none of the nine Proton records yet, removing any that runway created, since an identical record fails to create;
   - writes `workloads/noisypigeon.com/apex/secrets.config.yaml` with `pigeon cobot secrets set` for `protonmail_verification` and `pm_verification`, commits only that file to PR C's branch, and pushes.
4. **PR C**, `feat(workloads): noisypigeon.com apex with Proton Mail`, holds the apex's `config.yaml` from §2, the owner's secrets file, the `apex-noisypigeon.com` token, and `workloads/README.md`'s first zone.

### 14. Verification

- **Unit tests:**
  - presets expand to the expected records, a record with an unset input is skipped, and an override changes only its fields;
  - `false` drops a record, and an unknown `proton-*` or `host-*` key is rejected;
  - two presets claiming `@ MX`, a workload named `www` beside an apex workload, and an explicit `v=spf1` record beside Proton are each rejected, naming both owners;
  - an override that moves a host record onto another workload's host is rejected, and so is a `CNAME` sharing a non-apex name with an `MX`;
  - an apex assigned to a missing workload renders no host record and passes; a `hosts` entry naming a missing workload fails;
  - a workload with `ingress` renders its host records;
  - the layout rules of §1 each fail for their own reason;
  - a workload secret round-trips under the workload passphrase, a break-glass key under `workloads/` is rejected, and equal passphrases are refused;
  - the live generation is chosen correctly inside and after an overlap.
- **End to end**, with built-in-only fixtures: an infra unit whose outputs feed an apex-shaped unit plans and applies in one run, the handed-off values reach `tofu` but never `.cobot/` or the summary, and a pending handoff plans with the note but refuses to apply.
- **`tofu validate`** passes on the rendered `noisypigeon.com` apex against the real pinned provider.
- **PR C's plan comment** shows:
  - for `infra/cloudflare`, one `cloudflare_account_token` to create;
  - for `workloads/noisypigeon.com/apex`, nine `cloudflare_dns_record`s to create, with the pending-credentials note, and nothing to change or destroy.
- **After PR C merges:** the apply mints the token, then creates the nine records, commits encrypted state for both units, and the next plan shows "No changes" for both.
