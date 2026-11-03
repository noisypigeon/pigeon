# One R2 bucket; see .claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_r2_bucket" "this" {
  account_id    = var.account_id
  name          = var.name
  jurisdiction  = var.jurisdiction
  storage_class = var.storage_class
}
