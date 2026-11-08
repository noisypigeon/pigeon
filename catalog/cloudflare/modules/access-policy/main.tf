# One reusable Access policy; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md §4.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_zero_trust_access_policy" "this" {
  account_id = var.account_id
  name       = var.name
  decision   = length(var.service_token_ids) > 0 ? "non_identity" : "allow"
  include = concat(
    [for e in var.emails : { email = { email = e } }],
    [for t in values(var.service_token_ids) : { service_token = { token_id = t } }],
  )
}
