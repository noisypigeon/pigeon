# One private B2 bucket, named <key>-<suffix>; see .claude/decisions/2026-11-03-runtime-backblaze-provisioner-unit.md.
terraform {
  required_providers {
    b2 = {
      source  = "backblaze/b2"
      version = "0.14.0"
    }
  }
}

resource "b2_bucket" "this" {
  bucket_name = "${var.key}-${var.suffix}"
  bucket_type = "allPrivate"
}
