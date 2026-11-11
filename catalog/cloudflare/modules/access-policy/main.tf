# One reusable Access policy; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md §4.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

locals {
  tokens = concat(values(var.service_token_ids), var.service_token_id == null ? [] : [var.service_token_id])
}

resource "cloudflare_zero_trust_access_policy" "this" {
  account_id = var.account_id
  name       = var.name
  decision   = var.bypass ? "bypass" : length(local.tokens) > 0 ? "non_identity" : "allow"
  include = var.bypass ? [{ everyone = {} }] : concat(
    var.account_members ? [{ cloudflare_account_member = { account_id = var.account_id } }] : [],
    [for e in var.emails : { email = { email = e } }],
    [for t in local.tokens : { service_token = { token_id = t } }],
  )

  lifecycle {
    precondition {
      condition     = !var.bypass || (length(local.tokens) == 0 && length(var.emails) == 0 && !var.account_members)
      error_message = "A bypass policy admits everyone, so it takes no emails, service tokens or account members."
    }
  }
}
