# One zone ruleset; see .claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_ruleset" "this" {
  zone_id = var.zone_id
  name    = var.name
  kind    = "zone"
  phase   = var.phase
  rules   = var.rules
}
