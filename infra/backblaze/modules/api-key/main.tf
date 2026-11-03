# One generation of a bucket-scoped B2 application key; see .claude/decisions/2026-11-03-runtime-backblaze-provisioner-unit.md.
terraform {
  required_providers {
    b2 = {
      source  = "backblaze/b2"
      version = "0.14.0"
    }
  }
}

locals {
  unknown = setsubtract(var.buckets, nonsensitive(keys(var.bucket_ids)))
}

resource "b2_application_key" "this" {
  key_name                  = "${var.name}-${replace(substr(var.not_before, 0, 10), "-", "")}"
  capabilities              = var.capabilities
  bucket_ids                = [for b in var.buckets : lookup(var.bucket_ids, b, "")]
  valid_duration_in_seconds = (var.period_days + var.overlap_days) * 86400

  lifecycle {
    precondition {
      condition     = length(local.unknown) == 0
      error_message = "Keys name buckets missing from config.yaml's buckets: ${join(", ", sort(local.unknown))}."
    }
  }
}
