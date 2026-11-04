# Assigns resources to the project, nothing while there are none; see .claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md.
terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "2.106.0"
    }
  }
}

resource "digitalocean_project_resources" "this" {
  count     = length(var.urns) > 0 ? 1 : 0
  project   = var.project_id
  resources = values(var.urns)
}
