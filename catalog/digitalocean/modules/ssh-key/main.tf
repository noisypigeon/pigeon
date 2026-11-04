# One registered SSH public key; see .claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md.
terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "2.106.0"
    }
  }
}

resource "digitalocean_ssh_key" "this" {
  name       = var.name
  public_key = var.public_key
}
