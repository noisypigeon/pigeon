# One application's policy, a rule per entry; see .claude/decisions/2026-11-03-runtime-scaleway-provisioner-unit.md.
terraform {
  required_providers {
    scaleway = {
      source  = "scaleway/scaleway"
      version = "2.87.0"
    }
  }
}

locals {
  unknown = setsubtract(flatten([for r in var.rules : r.projects]), keys(var.project_ids))
}

resource "scaleway_iam_policy" "this" {
  name           = var.name
  application_id = var.application_id

  dynamic "rule" {
    for_each = var.rules
    content {
      permission_set_names = rule.value.permission_sets
      organization_id      = rule.value.organization ? var.organization_id : null
      project_ids          = rule.value.organization ? null : [for p in rule.value.projects : lookup(var.project_ids, p, "")]
    }
  }

  lifecycle {
    precondition {
      condition     = length(local.unknown) == 0
      error_message = "Rules name projects missing from config.yaml's projects: ${join(", ", sort(local.unknown))}."
    }
  }
}
