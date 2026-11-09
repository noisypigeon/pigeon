# One Access service token; its secret only in encrypted state; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md §4.
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

  # The live secret's version, and when the one before it stops being accepted; raising the
  # version rotates the secret.
  client_secret_version             = var.secret_version
  previous_client_secret_expires_at = var.previous_secret_expires
}
