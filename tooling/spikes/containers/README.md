# tooling/spikes/containers

The phase 3 spike: a Rust Worker and a Container built from the released
`pigeon-cobot` image, confirming what the control plane relies on, per the
phase 3 amendment of
[`2026-11-04-development-platform-instance-separation.md`](../../../.claude/decisions/2026-11-04-development-platform-instance-separation.md)
and the `tooling/spikes/` amendment of
[`2026-11-02-development-repository-area-layout.md`](../../../.claude/decisions/2026-11-02-development-repository-area-layout.md).

- `worker/`: the Worker (`workers-rs`), one bearer-gated route per check.
- `probe/`: the Container's check server on port 8080, and its `Dockerfile`.
- `wrangler.toml`: bindings with `SET_LOCALLY` placeholders; the real IDs and
  names live only in your gitignored `wrangler.local.toml`.

Nothing here is a release unit, and it is removed once the report is in the ADR.

## Deploy

In the **pigeon** account, with Docker running, `wrangler` 4, and the pinned
Rust from `mise` (with the `wasm32-unknown-unknown` target). Use a short-lived
token with Workers Scripts Edit, Containers Edit, D1 Edit, Workers R2 Storage
Edit and Secrets Store Edit, scoped to the pigeon account, and revoke it after
teardown.

```sh
cd tooling/spikes/containers
export CLOUDFLARE_API_TOKEN=…        # short-lived, pigeon account only
export CLOUDFLARE_ACCOUNT_ID=…       # never written to a tracked file

npx wrangler d1 create pigeon-spike                       # note the database_id
npx wrangler r2 bucket create "pigeon-spike-$(openssl rand -hex 4)"   # note the name
npx wrangler secrets-store store list                     # or `store create`; note its id
npx wrangler secrets-store secret create <store-id> --name pigeon-spike --scopes workers
#   (enter any test value; the spike reports only its length)

cp wrangler.toml wrangler.local.toml
#   replace the three SET_LOCALLY values in wrangler.local.toml

npx wrangler deploy -c wrangler.local.toml
openssl rand -hex 32 | tee /dev/stderr | npx wrangler secret put SPIKE_KEY -c wrangler.local.toml
export SPIKE_KEY=…                   # the value printed above
export SPIKE_URL=https://pigeon-spike.<your-subdomain>.workers.dev
```

## Run the checks

```sh
s() { curl -s -H "authorization: Bearer $SPIKE_KEY" "$SPIKE_URL$1"; echo; }
s /image      # first call is a cold start: tofu, zola and pigeon versions, startup_ms
s /egress     # every provider should report an HTTP status, not an error
s /secrets    # {"present":true,"length":N}
s /do; s /do  # count increments
s /r2         # read_back: true
s /run        # starts the long run; note uptime_s
#   wait 20 minutes, calling nothing, then:
s /uptime     # survived if uptime_s >= 1200 and cold_start is false
s /d1         # cron_ticks > 0 once 5 minutes have passed
```

Also note the image size (`docker image ls`) and the deploy's push time.

Paste every JSON result, and any deploy error, into the session; they become
the spike report.

## Tear down

```sh
npx wrangler delete -c wrangler.local.toml
npx wrangler d1 delete pigeon-spike
npx wrangler r2 object delete "<bucket>/spike/probe" --remote && npx wrangler r2 bucket delete <bucket>
npx wrangler secrets-store secret delete <store-id> --secret-id <secret-id>
rm wrangler.local.toml
```

Then revoke the token.
