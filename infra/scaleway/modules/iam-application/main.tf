# One IAM application, the principal of a policy and its keys; see .claude/decisions/2026-11-03-runtime-scaleway-provisioner-unit.md.
terraform {
  required_providers {
    scaleway = {
      source  = "scaleway/scaleway"
      version = "2.87.0"
    }
  }
}

resource "scaleway_iam_application" "this" {
  name        = var.name
  description = var.description
}
