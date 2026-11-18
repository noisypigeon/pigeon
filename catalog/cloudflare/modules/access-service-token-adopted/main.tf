# One Access service token the account already holds, read by name and never written, so no
# plan can rotate its secret; see
# .claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md, its 2026-11-18 amendments.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

data "cloudflare_zero_trust_access_service_tokens" "this" {
  account_id = var.account_id
  name       = var.name

  lifecycle {
    postcondition {
      condition     = length([for t in self.result : t if t.name == var.name]) == 1
      error_message = "The account must hold exactly one service token of this name."
    }
  }
}

locals {
  token = one([for t in data.cloudflare_zero_trust_access_service_tokens.this.result : t if t.name == var.name])
}
