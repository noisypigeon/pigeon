# A zone looked up by name; see .claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

data "cloudflare_zone" "this" {
  filter = {
    name  = var.name
    match = "all"
    account = {
      id = var.account_id
    }
  }
}
