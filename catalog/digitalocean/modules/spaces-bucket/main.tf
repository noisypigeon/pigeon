# One private Spaces bucket, named <key>-<suffix>; see .claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md.
terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "2.106.0"
    }
  }
}

resource "digitalocean_spaces_bucket" "this" {
  name          = "${var.key}-${var.suffix}"
  region        = var.region
  acl           = "private"
  force_destroy = false

  versioning {
    enabled = var.versioning
  }
}
