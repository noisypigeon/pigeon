# One Worker, looked up by name, so its ID never enters the repository; see
# .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md, step 4's amendment.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

data "cloudflare_worker" "this" {
  account_id = var.account_id
  worker_id  = var.name
}
