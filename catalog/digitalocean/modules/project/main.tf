# The account's one cobot project; see .claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md.
terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "2.106.0"
    }
  }
}

resource "digitalocean_project" "this" {
  name        = var.name
  description = var.description
  purpose     = var.purpose
  environment = var.environment
  is_default  = false
}
