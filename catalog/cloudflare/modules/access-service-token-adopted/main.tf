# One Access service token adopted unrotated: its secret left as Cloudflare holds it; see
# .claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md, 2026-11-18 amendment.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_zero_trust_access_service_token" "this" {
  account_id = var.account_id
  name       = var.name
  duration   = var.duration

  lifecycle {
    ignore_changes = [client_secret_version, previous_client_secret_expires_at]
  }
}
