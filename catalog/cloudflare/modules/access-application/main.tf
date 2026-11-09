# One self-hosted Access application; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md §4.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

locals {
  policies = concat(var.policy_ids, values(var.policy_id_map))
}

resource "cloudflare_zero_trust_access_application" "this" {
  account_id       = var.account_id
  name             = var.name
  type             = "self_hosted"
  domain           = var.domain
  session_duration = var.session_duration
  destinations     = var.worker_id == null ? null : [for t in var.worker_types : { type = t, worker_id = var.worker_id }]
  policies         = [for i, id in local.policies : { id = id, precedence = i + 1 }]

  # Login methods and these flags are the dashboard's; adopting an application keeps them.
  lifecycle {
    ignore_changes = [allowed_idps, auto_redirect_to_identity, enable_binding_cookie, options_preflight_bypass]
  }
}
