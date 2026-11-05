# tooling/spikes/containers

The phase 3 spike: a Rust Worker and a Container built from the released
`pigeon-cobot` image, confirming what the control plane relies on, per the
phase 3 amendment of
[`2026-11-04-development-platform-instance-separation.md`](../../../.claude/decisions/2026-11-04-development-platform-instance-separation.md)
and the `tooling/spikes/` amendment of
[`2026-11-02-development-repository-area-layout.md`](../../../.claude/decisions/2026-11-02-development-repository-area-layout.md).

- `worker/`: the Worker (`workers-rs`), one bearer-gated route per check.
- `probe/`: the Container's check server on port 8080, its TLS listener on
  8443 (when the `SPIKE_TLS_*` PEMs are set), and its `Dockerfile`.
- `wrangler.toml`: bindings with `SET_LOCALLY` placeholders; the real IDs and
  names live only in your gitignored `wrangler.local.toml`.

Nothing here is a release unit, and it is removed once the report is in the ADR.

## Deploy

In the **pigeon** account, on the Workers Paid plan (Containers need it), with
Docker running, `wrangler` 4, and the pinned Rust from `mise` (with the
`wasm32-unknown-unknown` target). Use a short-lived account API token, not the
`infra/pigeon` bootstrap token, with Account Settings Read, Workers Scripts
Edit, Containers Edit, D1 Edit, Workers R2 Storage Edit, Secrets Store Read|Write
and SSL and Certificates Read|Write (for the mTLS certificate), and revoke it after
teardown.

```sh
cd tooling/spikes/containers
export CLOUDFLARE_API_TOKEN=…        # short-lived, pigeon account only
export CLOUDFLARE_ACCOUNT_ID=…       # never written to a tracked file

npx wrangler d1 create pigeon-spike                       # note the database_id
npx wrangler r2 bucket create "pigeon-spike-$(openssl rand -hex 4)"   # note the name
npx wrangler secrets-store store list --remote            # the account's one store; note its id
npx wrangler secrets-store secret create <store-id> --name pigeon-spike --scopes workers --remote
#   (enter any test value; the spike reports only its length)

# A throwaway test CA, outside this checkout; nothing here is ever committed.
mkdir -p ~/pigeon-spike-tls && cd ~/pigeon-spike-tls
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 7 \
  -subj /CN=pigeon-spike-ca -keyout ca.key -out ca.pem
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=container \
  -keyout server.key -out server.csr
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -days 7 -out server.pem \
  -extfile <(printf 'subjectAltName=DNS:container\nextendedKeyUsage=serverAuth')
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=pigeon-spike-worker \
  -keyout client.key -out client.csr
openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key -days 7 -out client.pem \
  -extfile <(printf 'extendedKeyUsage=clientAuth')
openssl pkcs8 -topk8 -nocrypt -in server.key -out server.pk8
cd -

npx wrangler mtls-certificate upload --name pigeon-spike \
  --cert ~/pigeon-spike-tls/client.pem --key ~/pigeon-spike-tls/client.key
#   note the certificate id

cp wrangler.toml wrangler.local.toml
#   replace the four SET_LOCALLY values in wrangler.local.toml

npx wrangler deploy -c wrangler.local.toml
openssl rand -hex 32 | tee /dev/stderr | npx wrangler secret put SPIKE_KEY -c wrangler.local.toml
npx wrangler secret put SPIKE_TLS_CERT -c wrangler.local.toml < ~/pigeon-spike-tls/server.pem
npx wrangler secret put SPIKE_TLS_KEY -c wrangler.local.toml < ~/pigeon-spike-tls/server.pk8
npx wrangler secret put SPIKE_TLS_CLIENT_CA -c wrangler.local.toml < ~/pigeon-spike-tls/ca.pem
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
s "/run?minutes=20"   # starts a 20-minute busy job with a heartbeat every 30 s
#   wait about 22 minutes, calling nothing, then:
s /status     # survived if job.done, beats == expected_beats, longest_gap_s near 30,
              # and cold_start is false
s /d1         # cron_ticks > 0 once 5 minutes have passed
s /tls        # container_binding: a probe answer with "tls":true, or the error
s /mtls       # also mtls_binding; the probe's "client_certificate" says whether
              # the Worker presented the uploaded certificate
```

The Container reads the `SPIKE_TLS_*` secrets only when it starts, so set them
before the first call. `/mtls` is expected to fail or report
`"client_certificate":false`: an `mtls_certificates` binding presents its
certificate on fetches to the internet, and the container binding may not
accept one. Either result is what the report needs, per the mutual TLS
amendment's fallback order.

Also note the image size (`docker image ls`) and the deploy's push time.

Paste every JSON result, and any deploy error, into the session; they become
the spike report.

## Tear down

```sh
npx wrangler delete -c wrangler.local.toml
npx wrangler d1 delete pigeon-spike
npx wrangler r2 object delete "<bucket>/spike/probe" --remote && npx wrangler r2 bucket delete <bucket>
npx wrangler secrets-store secret delete <store-id> --secret-id <secret-id> --remote
npx wrangler mtls-certificate delete --name pigeon-spike
rm -r wrangler.local.toml ~/pigeon-spike-tls
```

Then revoke the token.
