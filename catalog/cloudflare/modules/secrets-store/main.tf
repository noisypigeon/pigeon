# One Secrets Store; see .claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md §1.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_secrets_store" "this" {
  account_id = var.account_id
  name       = var.name
}
