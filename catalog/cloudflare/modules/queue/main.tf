# One Queue; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_queue" "this" {
  account_id = var.account_id
  queue_name = var.name
}
