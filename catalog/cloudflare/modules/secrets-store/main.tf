# The account's one Secrets Store, read, never created; see
# .claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md, its 2026-11-14 amendment.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

data "cloudflare_secrets_stores" "this" {
  account_id = var.account_id

  lifecycle {
    postcondition {
      condition     = length(self.result) == 1
      error_message = "The account must hold exactly one Secrets Store."
    }
  }
}
