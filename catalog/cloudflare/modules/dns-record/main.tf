# One DNS record; see .claude/decisions/2026-11-02-runtime-workloads-zone-apex.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_dns_record" "this" {
  zone_id  = var.zone_id
  name     = var.name
  type     = var.type
  content  = var.content
  ttl      = var.ttl
  priority = var.priority
  proxied  = var.proxied
}
