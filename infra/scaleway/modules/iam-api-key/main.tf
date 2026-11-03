# One generation of an application's API key, expiring with it; see .claude/decisions/2026-11-03-runtime-scaleway-provisioner-unit.md.
terraform {
  required_providers {
    scaleway = {
      source  = "scaleway/scaleway"
      version = "2.87.0"
    }
  }
}

resource "scaleway_iam_api_key" "this" {
  application_id     = var.application_id
  default_project_id = var.default_project_id
  description        = "${var.name}@${substr(var.not_before, 0, 10)}..${substr(var.expires_on, 0, 10)}"
  expires_at         = var.expires_on
}
