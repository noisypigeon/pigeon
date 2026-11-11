## An instance is one config.yaml of kind-typed units

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `config.yaml`, `catalog/*/kinds/**`, `src/pigeon-cli/crates/backplane/src/cobot/manifest.rs`, `src/pigeon-cli/crates/backplane/src/cobot/unit.rs`, `src/pigeon-cli/crates/backplane/src/cobot/moves.rs`, `src/pigeon-cli/crates/backplane/tests/cobot_manifest.rs`

## Context

[`2026-11-05-development-pigeon-its-own-instance.md`](2026-11-05-development-pigeon-its-own-instance.md) moves noisypigeon to `noisypigeon/noisypigeon`, run from pigeon with `--instance`. Its workloads were built on special cases, which the move would carry with it:

- **Kinds come from paths.** `workloads/<zone>/apex` and a schema-less `workloads/<zone>/<w>` get catalog schemas composed by path. A generic unit cannot use a catalog definition at all: it carries its own `schema.json`, and `module: <catalog>/<name>` resolves only from `infra/<name>/`.
- **Credentials come by handoff.**
  - `zone-token` and `workload-token` (`$handoff.*`), and `per_zone` providers.
  - A token, zone ID and account ID read from `infra/cloudflare`'s state or, since the run-token change, from the API.
  - All of it exists because `infra/cloudflare` minted a token per workload, which the own-instance ADR §4 retires.
- **Secrets are referenced** (`secrets: "referenced"`) for values that are published anyway.
- **Units read each other's files.** A site's hosts come from reading the apex's `config.yaml` from disk.

The domain logic is sound and stays. That covers:
- presets, claims and record checks, per [`2026-11-02-runtime-workloads-zone-apex.md`](2026-11-02-runtime-workloads-zone-apex.md);
- buckets, custom domains, and the root-index and www rules, per [`2026-11-03-runtime-cloudflare-static-sites-r2.md`](2026-11-03-runtime-cloudflare-static-sites-r2.md);
- the template contract and the publish, per [`2026-11-03-runtime-static-site-config-contract.md`](2026-11-03-runtime-static-site-config-contract.md).

The owner asked that the first instance follow the engine's design, extended deliberately where it falls short, not the old leaves. They also asked that one `config.yaml` hold the instance's whole configuration, with any number of units for any purpose.

## Implementation plan.

### 1. The instance manifest, v1

An instance is its root `config.yaml`:

```yaml
schema: v1
name: noisypigeon
description: Willow Graysen's sites.
repository: https://github.com/noisypigeon/noisypigeon

units:
  zones/noisypigeon.com:
    kind: cloudflare/zone
    services: { proton: { verification: { protonmail: …, pm: … }, dmarc: default } }
    records: {}
  sites/blog:
    kind: cloudflare/static-site
    zone: noisypigeon.com
    hosts: ["@", www]
    template: { name: blog, version: 1.0.2, features: [math] }
    content: sites/blog
    site: { title: …, … }

moves:
  - { from: workloads/noisypigeon.com/apex, to: zones/noisypigeon.com }
  - { from: workloads/noisypigeon.com/blog, to: sites/blog }
```

- **Unit IDs.**
  - Each key under `units` is a unit's ID, conventionally `<area>/<name>`, matching `^[a-z0-9]+(-[a-z0-9]+)*(/[a-z0-9]+([.-][a-z0-9]+)*)+$`: no segment is empty, starts or ends with `.` or `-`, or is `.` or `..`.
  - A `moves` entry's `from` and `to` follow the same pattern. A unit's state path, joined under the state root, must stay inside it, or the unit is refused.
  - The ID is the unit's state path, and its row in D1 `desired_state` ([`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md) §4).
- **A unit's entry** is its `kind` plus its settings, validated by the kind's schema.
- **`moves`** moves a unit's state from one ID to another, through the existing moves machinery. It is removed once applied.
- **Content.** Files outside `config.yaml` are content only, named by a unit's settings (here `content: sites/blog`, holding `posts/`, `pages/` and `assets/`).
- **What the instance never holds:** a secret, ID, bucket name, state, schema, module or workflow.
- **What it replaces.**
  - It replaces `.config/instance.yaml` and per-directory `config.yaml` units for an instance.
  - pigeon's own directory units (`infra/*`, `unit.yaml`) keep working until they move in a later step.
  - `.config/instance.yaml` stays readable as manifest `v0`.

### 2. Kinds

- **Where a kind lives.** A kind is defined by the platform, in `catalog/<catalog>/kinds/<kind>/schema.json`. It is a JSON Schema with the generic `x-cobot` model: properties, `singletons`, `each`, `outputs`, `providers` and `credentials`.
- **Naming one.** A unit names its kind as `kind: <catalog>/<kind>`, resolved from the platform root.
- **Module lookup.** `module: <catalog>/<name>` resolves against the platform's `catalog/`, from any unit, at any depth.
- **Expanders.** A kind may declare `derived` blocks with a named expander, such as `zone-records`, `zone-rulesets`, `site-bucket` or `site-domains`. That replaces the apex and workload path rules. An expander sees its unit's settings and the instance manifest, never another file.
- **The first two kinds:**

  | Kind | Settings | Blocks | Credentials |
  | --- | --- | --- | --- |
  | `cloudflare/zone` | `services` (presets), `records`; the zone is the unit's name | `account` and `zone` lookups; `dns_records` (presets, records, and claims across the manifest); `zone_rulesets` (the root-index rewrite and the www redirect for the manifest's sites in this zone) | `dns.write`, `transform.write` and `redirects.write` on the zone |
  | `cloudflare/static-site` | `zone`, `hosts`, `template`, `content`, `site`, `storage_class` | `account` and `zone` lookups; `bucket`; `domains`, one per host | `r2.write` on the account (`r2.bucket.write` on its bucket when that lands); `zone.read` and `dns.write` on its zone |

- **Hosts are claimed by the site that serves them.** The zone checks them across the manifest against its records and presets, under the zone-apex ADR's claim rules.
- **Preset inputs marked `published`**, such as the Proton verification codes, are plain values, never secrets.

### 3. Credentials

- **Declared, never held.** `x-cobot.credentials` entries gain `target`: the scope's subject, an expression such as the zone or `$config.zone`. A kind's provider takes `$credential.<name>.api_token`.
- **Filled by the run.**
  - When `COBOT_CREDENTIALS` doesn't name a requested credential, a run fills every credential of that provider from its own token: `CLOUDFLARE_API_TOKEN` when run by hand, under the own-instance ADR §3.
  - The control plane later mints each credential exactly, from its capabilities, scope and target (the control-plane ADR §8).
- **Looked up, never written.** Account and zone IDs come from the `cloudflare/account` and `cloudflare/zone` modules.
- **What goes:** no handoff, no `per_zone` and no workload secrets. They are removed with `infra/cloudflare` (§6).

### 4. State continuity

- **Addresses are kept.** The kinds keep the existing block names and item keys (`dns_records["<zone>:<key>"]`, `zone_rulesets["<zone>:<name>"]`, `bucket["site"]`, `domains["<label>"]`), so a moved unit's resources keep their addresses.
- **The first plan.** With the manifest's `moves`, the first plan of `noisypigeon/noisypigeon` destroys and replaces nothing.

### 5. Operations

- `infra.validate`, `infra.plan`, `infra.apply`, `sites.build` and `architecture.describe` read the manifest's units.
- `sites.publish` joins backplane's catalog, so a site's publish goes through the single entry point too.

### 6. Supersedes

The superseded texts stay as written:

- **The path layout and the handoff**, in:
  - [`2026-11-02-runtime-workloads-zone-apex.md`](2026-11-02-runtime-workloads-zone-apex.md);
  - [`2026-11-03-runtime-cloudflare-static-sites-r2.md`](2026-11-03-runtime-cloudflare-static-sites-r2.md);
  - [`2026-11-02-runtime-cloudflare-provisioner-unit.md`](2026-11-02-runtime-cloudflare-provisioner-unit.md)'s per-workload minting.
- **`workloads/` as an instance area**, in [`2026-11-02-development-repository-area-layout.md`](2026-11-02-development-repository-area-layout.md).
- **The own-instance ADR's file list** for `noisypigeon/noisypigeon`, in §1 and §6 step 4.

### 7. Execution

Each step is released before the next relies on it.

1. This ADR, and its `CLAUDE.md` pointers.
2. The two roots, per the own-instance ADR's amendment. This is merged as #135, and its preset half follows: Proton's verification inputs marked `published`.
3. `feat(cobot)`: manifest v1, kinds, kind-declared expanders, credential targets and the fill from the run token, and the manifest's `moves`.
4. `feat(catalog)`: `cloudflare/zone` and `cloudflare/static-site`.
5. `feat(backplane)`: the v1 instance, and `sites.publish`.
6. `noisypigeon/noisypigeon`'s first commit: `config.yaml`, `sites/blog/` and `.gitignore`.
7. **The owner, by hand, from pigeon with `--instance`:**
   - plan, which shows no destroy or replace, and apply;
   - publish;
   - check the blog;
   - remove `moves`;
   - revoke `infra/cloudflare`'s tokens.
8. **This repository's cleanup**, per the own-instance ADR §6 steps 6 and 7. It also removes the handoff, `per_zone`, path-based composition and workload secrets.

### 8. Verification

- **Address preservation.** A test renders today's apex and blog units and the new zone and site units, and finds the same module addresses and inputs.
- **The moved plan.** A `tofu` plan of moved state, with a fake Cloudflare API, shows 0 to destroy and 0 to replace.
- **IDs.** A unit ID or move with a `.` or `..` segment, or one whose state path leaves the state root, is refused.
- **Hosts.** A host claimed by two sites, or by a site and a zone record, is refused.
- **Credentials.** A run token fills every declared `cloudflare` credential, and appears in no rendered or state file.
- **The repository.** `noisypigeon/noisypigeon` holds no `ENC[`, ID or bucket name.
- **The owner's plan** destroys and replaces nothing, and the blog and mail keep working.
