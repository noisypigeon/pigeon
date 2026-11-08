# One self-hosted Access application; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md §4.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_zero_trust_access_application" "this" {
  account_id       = var.account_id
  name             = var.name
  type             = "self_hosted"
  domain           = var.domain
  session_duration = var.session_duration
  policies         = [for i, id in var.policy_ids : { id = id, precedence = i + 1 }]
}
