# The account-scoped permission groups; see .claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

data "cloudflare_account_api_token_permission_groups_list" "this" {
  account_id = var.account_id
}
