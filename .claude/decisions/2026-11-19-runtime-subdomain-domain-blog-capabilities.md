## Subdomain, Domain and Blog: three capabilities, first enabled on the noisypigeon instance

* Date: 2026-11-19
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/**/worker/hosts.rs`, `src/pigeon-core/src/platform/hosts.rs`, `src/pigeon-core/src/infra/private.rs`

## Context

[`2026-11-19-runtime-capability-interface-manifest-trait.md`](2026-11-19-runtime-capability-interface-manifest-trait.md)
gives every capability a manifest and a trait. The owner answered what each
of these three means in
[`2026-11-18-security-users-levels-capability-grants.md`](2026-11-18-security-users-levels-capability-grants.md)
(answers 39 to 41):

- **Subdomain:** a user publishes their own static site at
  `<label>.pigeon.dev`.
- **Domain:** a user attaches a domain they own to a site or instance they
  hold.
- **Blog:** a user's own blog, from the blog template, deployed to a
  subdomain or to a domain's apex.

This builds on:

- [`2026-11-15-runtime-standalone-hostnames-platform-zone.md`](2026-11-15-runtime-standalone-hostnames-platform-zone.md):
  the platform's wildcard `*.pigeon.dev` route, `check_label`, and
  `worker/hosts.rs`, which forwards a known host's `/v0/app/*` to its
  standalone. This is live (!1304).
- [`2026-11-15-runtime-zoned-standalone-account-imports.md`](2026-11-15-runtime-zoned-standalone-account-imports.md):
  private zones by alias, `zones.private`, `zone import`, `instance.plan`
  and `instance.apply`, and noisypigeon's repository (§7) and cutover (§8).
  Its steps 2, 3, 4 and 6 have landed (!1316, !1338, !1339, !1346, !1320,
  !1328, !1332, !1318). Its step 5 is a prerequisite here.
- [`2026-11-03-runtime-cloudflare-static-sites-r2.md`](2026-11-03-runtime-cloudflare-static-sites-r2.md)
  and the blog template (`src/pigeon-site-templates/blog/`): a static site
  is an R2 bucket behind R2 custom domains, built with Zola.
- [`2026-11-13-runtime-non-platform-profiles.md`](2026-11-13-runtime-non-platform-profiles.md):
  a standalone runs in its own account.

**The noisypigeon instance** is the zoned standalone in the noisypigeon
Cloudflare account that §7 of the zoned ADR describes, holding the blog. It
is the first instance to enable these capabilities. Willow is its only
member and its Administrator, so no grant is made for it.

**Privacy.** Labels, domains and zone names are private. They never enter
the repository, a change, a branch, a log, an event or a fixture. Code and
tests use `example.com` and `example.pigeon.dev`.

## Implementation plan.

### 1. Private scopes

A grant's scope appears in `member.granted` and `member.revoked` events and
on the Members screen. For `subdomain` and `domain`, the scope would be a
label or a domain.

- The manifest's `scope.private: true` makes the scope an **alias**:
  `[a-z0-9-]`, no dot, mapped to its value in the instance's `Instance`
  object. A domain's alias is its zone alias from `zones.private`. A
  subdomain's alias is the site's.
- Events, logs and the digest carry only the alias. The Members screen
  resolves it for the Administrator.
- `blog` and `icbc` scopes are not private.

### 2. Subdomain

| | |
| --- | --- |
| Scope | `dns_label`, private: one site alias, whose label is checked by `hosts::check_label` |
| Operations | `subdomain.claim {site, label}`, `subdomain.release {site}`, `subdomain.show {site}` |
| Storage | `hosts.private` entries `{alias, label, target}` in the platform's `Instance`; site files in the platform's sites bucket under `subdomain/<alias>/` |
| Routes | none of its own; `worker/sites_host.rs` serves the files |
| Grantable | yes, at the end of step 4 |

- **Serving.** `worker/hosts.rs` already looks a host up. Its answer gains a
  kind:
  - `instance`: forwarded to the standalone's `origin`, unchanged;
  - `site`: served by `worker/sites_host.rs` from the platform's sites
    bucket at `subdomain/<alias>/`, with `/` read as `/index.html` and
    `www.<label>` never served.
  - The 60-second cache, the bare 404 and "nothing logs a host" all stay.
- **Claiming** checks the label and writes the entry: owner tier for the
  Administrator, and device-signed for a Standard member holding the grant.
  The answer names the rule that failed, never another site's label.
- **Publishing** a site's files is `sites` (a later ADR) or `blog` (§4). A
  `subdomain` grant alone gives an empty, claimed host.
- **Released** labels are kept for 30 days before anyone else may claim
  them.

### 3. Domain

| | |
| --- | --- |
| Scope | `domain`, private: one zone alias in the instance's `zones.private` |
| Operations | `domain.attach {zone, target, hosts}`, `domain.detach {zone, target}`, `domain.show {zone}` |
| Storage | `zones.private` (the zoned ADR's), plus `domains.attached` `{zone, target, hosts}` in the instance's `Instance` |
| Secrets | none; tokens are minted per run from the account's `granter` (zoned ADR §3) |
| Grantable | yes, at the end of step 5 |

- **The zone must be in the instance's own account.** A domain in an account
  the instance can't mint for is out of scope. It moves accounts as the zoned
  ADR's §8 step 5 has it.
- **Attaching** writes the target's site unit with `zone: <alias>` and
  `hosts`, in the instance's private config. `instance.plan` renders the R2
  custom domains and the apex rulesets, and `instance.apply` (owner tier)
  applies them. A first plan that would replace or delete a record is
  refused, as the zoned ADR's §5 has it.
- **A target** is a site (`blog` or, later, `sites`) or an instance the
  grantee holds. For an instance, attaching adds the host to its private
  listing. That is out of scope until a second hostname per instance is
  designed, so the validator refuses an instance target for now.
- **Detaching** removes the custom domains and keeps the zone and its other
  records.

### 4. Blog

| | |
| --- | --- |
| Scope | `site`: one blog's name, `[a-z0-9-]` |
| Needs | `subdomain` or `domain`, covering the blog's host |
| Operations | `blog.create {blog, template_version}`, `blog.show {blog}`, `blog.post.put {blog, path, body}`, `blog.post.delete {blog, path}`, `blog.publish {blog}` |
| Storage | the instance's repository, `sites/<blog>/` (`config.yaml`, `site/posts`, `site/pages`, `site/assets`) |
| Config | `{template_version}`, defaulting to the template's current one |
| Grantable | yes, at the end of step 6 |

- **The source is the instance's repository**, per the zoned ADR's §7. A
  post is a commit on its Artifacts `main`, made by the control plane with
  the author's account in the trailer. It is not a pigeon change, because
  the repository is the instance's own.
- **`blog.publish`** runs the zoned ADR's `instance.apply` site step for one
  site: Zola builds it on pigeon's Runner, and it syncs to the target bucket
  and checks `/` and `/feed.xml` live.
  - For a subdomain, the target is `subdomain/<alias>/` in the platform's
    sites bucket.
  - For a domain, it is the site's own bucket in the instance's account.
- **Behaviour hooks:** `Before(blog.post.put)` checks front matter against
  the template's schema and refuses a path outside `site/`.
  `After(blog.publish)` records the published commit.
- **Who publishes.** The Administrator, from the app or MCP, without a
  passkey on each post: publishing a site they already hold is device-signed,
  the learner ADR's §5 pattern. Enabling a blog, and its first publish to a
  new host, take a passkey.
- **Agents** never call `blog.*`. They change the blog template through
  pigeon changes (the first amendment's §2).

### 5. Enabled on noisypigeon

The noisypigeon instance's `capabilities`, in its `Instance` object:

```yaml
domain: { version: 1.0.0 }      # its two zone aliases
blog:   { version: 1.0.0, config: { template_version: 1.0.2 } }
```

- `sites/blog` targets the `blog` zone alias at `@` and `www`, as the
  legacy workload does today.
- `subdomain` is not enabled there, because the blog keeps its own domain.

### 6. Steps

| Step | What | Who | After |
| --- | --- | --- | --- |
| 1 | This ADR | planner | the capability ADR |
| 2 | Zoned ADR steps 2, 3, 4 and 6 | briefs | done |
| 3 | Zoned ADR step 5: `zones.private` operations, `/v0/app/mint`, `instance.plan`, `instance.apply` | briefs | — |
| 4 | Subdomain: manifest, `hosts.private` kinds, `sites_host.rs`, `subdomain.*`, private scopes (§1) | brief; the Worker route reviewed by the planner | capability steps 2–3 |
| 5 | Domain: manifest, `domain.*` | brief | 3 |
| 6 | Blog: manifest, `blog.*`, the instance-repo commit path, `blog.publish` | brief | 3 |
| 7 | Each capability's `grantable: true` and its Members entry (`core/member`) | planner | 4, 5, 6 |
| 8 | The owner's rollout to noisypigeon (§7) | owner, with the planner | 1–7 |

### 7. The owner's rollout to noisypigeon

Filed with `owner_ask` once steps 1 to 7 have merged and deployed. In order,
each step's check gating the next:

1. **Granter and state passphrase** in the noisypigeon account's Secrets
   Store, per the zoned ADR's §9 (steps 1 to 5 there).
2. **The repository:** create `github.com/noisypigeon/noisypigeon`, empty,
   and add it to the `pigeon-mirror` App's installation.
3. **Stand-up** from the app: label, the one-time pasted token and a
   passkey. The instance has shape `app` with `domain` and `blog` enabled.
4. **Inventory** (`instance.plan`, read-only): both zones are imported into
   `zones.private`. The planner checks for `unmanaged` items and that the
   plan shows no DNS change.
5. **Approve `instance.apply`** with a passkey: it adopts the bucket and
   custom domains in place and publishes `sites/blog` into the same bucket.
   The live check fetches `/` and `/feed.xml` on each host.
6. **Retire** the legacy workload in pigeon: one change, proposed by the
   planner and approved by the owner.

Nothing is cut over before the owner's step 5, and every step before it is
read-only on live DNS.

### 8. Tests

- `check_label` and the alias rules: well-formed, reserved, taken, and the
  refusal never naming another site.
- `covers`: a grant scoped to one alias covers no other. Blog's `needs` is
  refused without `subdomain` or `domain`.
- `sites_host.rs`: a site host serves its prefix, an instance host still
  forwards, an unknown host is a bare 404, and no host reaches a log line.
- `domain.attach`'s plan on an imported fixture zone (`example.com`) shows
  no record change.
- `blog.post.put` refuses a path outside `site/`, and bad front matter.

### 9. Rollback

- Each capability is disabled per instance (`capability.disable`), which
  removes its routes and keeps its data.
- The legacy workload stays until rollout step 6. Until then the blog is
  served by what serves it today.
