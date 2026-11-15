## Zoned standalones: private zones, imported DNS, per-instance repositories and a granter per account

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `catalog/cloudflare/kinds/zone/schema.json`, `catalog/cloudflare/kinds/git-repo/schema.json`, `src/pigeon-cli/crates/backplane/src/cobot/apex.rs`, `src/pigeon-cli/crates/backplane/src/cobot/adopt.rs`, `src/pigeon-cli/crates/backplane/src/cobot/git.rs`, `infra/cloudflare/config.yaml`

## Context

The owner has three Cloudflare accounts, and Cloudflare Organizations are not
available to them, so everything here is designed per account:

- **noisypigeon** owns two zones, one of them the blog's.
- **A personal account** owns the owner's personal domain.
- **The learner's account** has no zone and is served at `<label>.pigeon.dev`.
  It is built under
  [`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
  and
  [`2026-11-15-runtime-standalone-hostnames-platform-zone.md`](2026-11-15-runtime-standalone-hostnames-platform-zone.md),
  and is out of scope here.

**The owner's rule:** every zone other than `pigeon.dev` is private. A zone's
name, its records and their data never enter git, nor a branch, a change's
title or plan, a log, or a fixture. The rule covers all three accounts.

- Account names are private too, except `noisypigeon`, which is the public
  GitHub organization.
- Code and tests use `example.com`, `example.dev` and `example.pigeon.dev`.

### What exists

- **The blog** sits in this repository under the legacy layout:
  - a workload directory named after its zone, holding the zone's records;
  - Proton at the zone's apex, per
    [`2026-11-13-runtime-zone-vendor-dns-placements.md`](2026-11-13-runtime-zone-vendor-dns-placements.md);
  - tokens in `infra/cloudflare/config.yaml`;
  - applied by hand into the noisypigeon account, with state in the orphaned
    `pigeon-cobot-state`.

  Real zone names already appear in tracked config, code, tests, policies,
  merged ADRs and changelogs.
- **A standalone** is `shape: app` (learner ADR §1, and its stand-up
  amendment): a control Worker, its `Instance` object and one R2 bucket, with
  no Containers, D1 or Access, and listed through `instances.private`. The
  stand-up itself is another planner's work and is not built yet.
- **Artifacts repositories for any instance:** the `cloudflare/git-repo` kind,
  and `pigeon cobot git apply|status|mirror|sync --instance DIR`, which the
  owner's CLI runs with a token for that account.
- **A granter** is the platform account's Secrets Store secret `granter`, per
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md).
  No other account has one.
- **Zone units** support `A`, `AAAA`, `CNAME`, `MX` and `TXT`. `plan --adopt`
  doesn't adopt DNS records, and live records can't be imported.

## Implementation plan.

### 1. Private zones

**A tracked config names a private zone only by an alias.**

```yaml
zones/blog:                    # the alias; the zone's name is private
  kind: cloudflare/zone
  private: true                # services and records come from the private store
sites/blog:
  kind: cloudflare/static-site
  zone: blog                   # an alias, resolved at render
  hosts: ["@", www]
```

- A unit with `private: true` has no `services` or `records`. The schema
  refuses either one.
- A zone unit's key and every `zone:` reference are either `pigeon.dev` or an
  alias, which is a key of `[a-z0-9-]` with no dot.

**The private store** is `zones.private` in the instance's own `Instance`
Durable Object: the platform's for pigeon, a standalone's for itself.

- **An entry** is `{alias, zone, services, records, updated}`.
- **To write one,** the owner runs `zone.private.set <alias>` with the unit's
  YAML: owner tier, with a passkey, and it opens no change.
- **To read one,** `zone.private.show <alias>` is agent tier and shows the
  records. The page and response are behind Access or the app's session, and
  are never logged.

**Rendering:**

- `pigeon cobot render|plan|apply --private FILE` merges each entry into its
  alias's unit before checks and expansion.
- `FILE` is JSON. A job writes it to its temporary directory and never under
  the instance root.
- Without `--private`, a `private: true` unit renders nothing and plans
  nothing. A tracked-only check passes.

**Logs and plan text:**

- A job replaces every private zone name with `<alias>` in its stdout, its
  stderr and its stored plan summary.
- The stored plan (the private R2 plans bucket) keeps the real values, and only
  the owner's approval page decrypts them.

### 2. Enforcing the rule

- **Locally**, a sentinel policy `.config/policies/zones.yaml` fails a change
  in either of two cases:
  - its tracked config or catalog names a zone unit key, `zone:` value or
    `hosts` entry with a dot that isn't `pigeon.dev`, a subdomain of
    `pigeon.dev`, or under `example.com`, `example.dev` or `example.pigeon.dev`;
  - a file under `src/**/tests/**` names any domain outside that allowlist and
    `github.com`.
- **On the platform,** the `bound` job also scans each change's added lines for
  every name in the instance's `zones.private`, plus each standalone's whose
  `zones.private` the platform can read through §3. On a match it fails with
  `names a private zone (<alias>)`, and never prints the name.
- **Already committed:** this ADR doesn't rewrite history, merged ADRs or
  changelogs. Tracked files that name a private zone are scrubbed going forward
  (§8 step 4). The owner decides whether history matters.

### 3. A granter in every account

Every account holds its own `granter` in its own default Secrets Store. It is
an account-owned API token with *Account API Tokens Write* only, made by the
owner (§9).

A standalone's control Worker binds it as `GRANTER` and answers
`POST /v0/app/mint`:

- **The caller** is the platform's control Worker, signing the request with the
  stand-up's `HANDOVER` key pair. The request names a job ID and a capability
  set drawn from the units' `x-cobot.credentials`.
- **The answer** is sealed to the job's public key, so the platform Worker
  never reads it. It holds:
  - a token with exactly those capabilities, limited to the units' zones, with
    a TTL of one hour or less;
  - the private zone entries the job's units name;
  - the standalone's `state_passphrase`.
- **Every mint** is logged in the standalone's `Instance` object with its job
  ID. The token itself is never stored.

pigeon holds no standing credential for another account.

The personal account gets a granter and nothing else for now. Its instance and
zone are later enrolled entirely through the private stores.

### 4. `instance.plan` and `instance.apply`

**Platform jobs on pigeon's Runner, for a standalone's units:**

- `instance.plan` is agent tier and read-only. `instance.apply` is owner tier,
  with a passkey.
- **Each job:**
  1. clones the standalone's repository from its Artifacts `main`;
  2. mints through §3;
  3. writes the private entries to `--private`;
  4. runs `pigeon cobot plan --adopt` or `apply` with `--instance` set to the
     clone;
  5. publishes the sites whose inputs changed;
  6. runs `cobot git mirror`.
- **State** lives in the standalone's bucket under `state/`, encrypted with
  `state_passphrase`.
- **The first run** is before the granter is bound, so it uses the stand-up's
  one-time pasted token.

### 5. Importing a zone's live records

**`pigeon cobot zone import <alias> [--zone NAME] [--from FILE] [--instance DIR]`:**

- **Its input** is the zone's records, read from the API (*Zone Read*, *DNS
  Read*) or from a BIND export.
- **It removes** the records a declared preset or site already renders.
- **It keys the rest** `<label>-<type>`, or `apex-<type>` at `@`, with a
  numeric suffix where keys collide.
- **It writes** `records:` YAML to stdout, for `zone.private.set`. As a
  platform job it writes the YAML straight into the store, with the owner's
  passkey.
- **Anything outside DNS** is listed as `unmanaged`, and the command exits 3:
  rulesets, page rules, redirects, Email Routing, Workers routes and custom
  domains, R2 custom domains, and DNSSEC. Each gets support (an amendment)
  before cutover.
- **Never in the output:** record, zone and account IDs.

**`plan --adopt` adopts DNS records:**

- It matches name, type and content, or `data`, and writes the import ID
  `<zone_id>/<record_id>` only in the import file under `--state`.
- The first plan of an imported zone must show no change. If it would replace
  or delete any record, the apply is refused.

### 6. Record types

| Type | Fields |
| --- | --- |
| `SRV` | `data: {priority, weight, port, target}` |
| `CAA` | `data: {flags, tag, value}`, where `tag` is `issue`, `issuewild` or `iodef` |
| `NS` | `content`, refused at `@` |

- Every record gains `comment`, of at most 100 characters.
- `content` and `data` are mutually exclusive.
- `CAA` claims its name.
- `NS` delegates its subtree: no other record may sit at or under that name.
- A bare `*` name and proxied `AAAA` stay valid.

### 7. Repositories per instance

- An instance's Artifacts namespace is its name, in its own account. Its
  repositories are `cloudflare/git-repo` units.
- `mirror` names a GitHub repository. `instance.apply` ends by pushing to it
  with a `pigeon-mirror` App installation token
  ([`2026-11-08-development-github-mirror-on-merge.md`](2026-11-08-development-github-mirror-on-merge.md)),
  minted per run for that one repository.
- The git-repo kind gains `mirror_app`: `pigeon-mirror` (the default) or `none`
  (the owner's CLI only).
- noisypigeon has one repository, `noisypigeon`, mirrored to
  `github.com/noisypigeon/noisypigeon`. It holds:
  - `config.yaml`, with `repos/noisypigeon`, two private zone aliases,
    `sites/blog` and `platform/main` (`shape: app`);
  - `sites/blog/`, the blog's posts, pages and assets, moved unchanged.

  Its site config names no zone.

### 8. Cutover, with no downtime

The order is fixed. Each step's check gates the next.

1. **Inventory.** `instance.plan` imports both zones into noisypigeon's
   `zones.private` with a read-only minted token. It reports which account
   holds each zone, anything `unmanaged`, and the blog's bucket and custom
   domains. Any unsupported item is added (§5, §6) before going on.
2. **Declare.** `plan --adopt` must show no DNS change, and must adopt the
   bucket and its custom domains in place.
3. **Publish.** `instance.apply` publishes `sites/blog` into the same bucket.
   - Its live checks fetch `/` and `/feed.xml` on each host.
   - The content is unchanged apart from the build time.
4. **Retire and scrub.** In pigeon:
   - remove the legacy workload directory and its two tokens;
   - remove the secrets policies' pins to it;
   - replace each real zone name in tracked code and tests with `example.com`.

   Merged ADRs and changelogs keep their text.
5. **A zone in another account** moves as its own owner-approved step, with
   steps 2 and 3 run on the new account first:
   1. add the zone, which stays pending;
   2. apply every record;
   3. turn DNSSEC off;
   4. switch the nameservers.

### 9. The owner's per-account guide

For each account:

1. **Secrets Store:** Dashboard → the account → *Secrets Store*. Create the
   store if there is none (an account has one).
2. **Account API Tokens:** Manage account → *Account API Tokens* → *Create
   Token* → *Custom*:
   - name `granter`;
   - permission *Account* · *Account API Tokens* · *Edit*;
   - no zone resources, no IP filter;
   - TTL 90 days.
3. **Store the token:** *Secrets Store* → *Create secret*:
   - name `granter`;
   - scope *Workers*;
   - value the token.

   Paste it nowhere else.
4. **Workers subdomain:** *Workers & Pages* → set the `workers.dev` subdomain,
   if it isn't set.
5. **State passphrase** (standalone accounts): a secret `state_passphrase` in
   the same store, scope *Workers*, with the value of
   `openssl rand -base64 32`.

### 10. Steps

| Step | What | Who |
| --- | --- | --- |
| 1 | This ADR | planner |
| 2 | Zone record types (§6) | brief |
| 3 | `zone import` and DNS adoption (§5) | brief |
| 4 | Private zones: schema, `--private`, the policy and log masking (§1, §2) | brief |
| 5 | `zones.private` operations, `/v0/app/mint`, `instance.plan` and `instance.apply` (§1, §3, §4) | briefs, after the learner stand-up job |
| 6 | git-repo `mirror_app` and mirror minting (§7) | brief |
| 7 | Granters in all three accounts (§9) | owner |
| 8 | Create `github.com/noisypigeon/noisypigeon`, empty, and add it to the App's installation | owner |
| 9 | Stand up noisypigeon from the app | owner |
| 10 | Cutover (§8) | planner, with owner approvals |
