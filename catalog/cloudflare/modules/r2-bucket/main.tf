# One R2 bucket, optionally named <name>-<suffix>; see .claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md
# and .claude/decisions/2026-11-04-development-platform-instance-separation.md.
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
  name          = var.suffix == null ? var.name : "${var.name}-${var.suffix}"
  jurisdiction  = var.jurisdiction
  storage_class = var.storage_class
}
