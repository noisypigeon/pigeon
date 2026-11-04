# The one account the provider's token sees; see .claude/decisions/2026-11-04-development-platform-instance-separation.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

# Two items are enough to tell one account from several.
data "cloudflare_accounts" "this" {
  max_items = 2
}

locals {
  ids = distinct([for a in data.cloudflare_accounts.this.result : a.id])
}
