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

# `cloudflare_worker`'s `id` echoes its input; the list carries the immutable ID.
data "cloudflare_workers" "all" {
  account_id = var.account_id
}

locals {
  id = one([for w in data.cloudflare_workers.all.result : w.id if w.name == var.name])
}
