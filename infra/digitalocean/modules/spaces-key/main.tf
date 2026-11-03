# One generation of a bucket-scoped Spaces key; see .claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md.
terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "2.106.0"
    }
  }
}

locals {
  unknown = setsubtract([for g in var.grants : g.bucket], nonsensitive(keys(var.bucket_names)))
}

resource "digitalocean_spaces_key" "this" {
  name = "${var.name}@${substr(var.not_before, 0, 10)}..${substr(var.expires_on, 0, 10)}"

  dynamic "grant" {
    for_each = var.grants
    content {
      bucket     = lookup(var.bucket_names, grant.value.bucket, "")
      permission = grant.value.permission
    }
  }

  lifecycle {
    precondition {
      condition     = length(local.unknown) == 0
      error_message = "Grants name buckets missing from config.yaml's buckets: ${join(", ", sort(local.unknown))}."
    }
  }
}
