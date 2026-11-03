# One private Spaces bucket, named <prefix>-<key>; see .claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md.
terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "2.106.0"
    }
  }
}

resource "digitalocean_spaces_bucket" "this" {
  name          = "${var.prefix}-${var.key}"
  region        = var.region
  acl           = "private"
  force_destroy = false

  versioning {
    enabled = var.versioning
  }
}
