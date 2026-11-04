# One D1 database; see .claude/decisions/2026-11-04-development-platform-instance-separation.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_d1_database" "this" {
  account_id            = var.account_id
  name                  = var.name
  primary_location_hint = var.location
}
