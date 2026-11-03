# One random string of lowercase letters and digits; see .claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md.
terraform {
  required_providers {
    random = {
      source  = "hashicorp/random"
      version = "3.9.1"
    }
  }
}

resource "random_string" "this" {
  length  = var.length
  lower   = true
  numeric = true
  upper   = false
  special = false
  keepers = var.keepers
}
