# One private R2 bucket, named <key>-<suffix>; see .claude/decisions/2026-11-04-development-platform-instance-separation.md.
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
  name          = "${var.key}-${var.suffix}"
  jurisdiction  = "eu"
  storage_class = "Standard"
}
