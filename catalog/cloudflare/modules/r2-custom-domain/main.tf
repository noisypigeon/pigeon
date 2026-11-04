# One custom domain serving an R2 bucket; see .claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_r2_custom_domain" "this" {
  account_id   = var.account_id
  bucket_name  = var.bucket_name
  jurisdiction = var.jurisdiction
  domain       = var.domain
  zone_id      = var.zone_id
  enabled      = var.enabled
  min_tls      = var.min_tls
}
