# One generation of a minted account token; see .claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

locals {
  scope = {
    account = "com.cloudflare.api.account"
    zone    = "com.cloudflare.api.account.zone"
  }
  unknown_zones = setsubtract(flatten([for c in var.capabilities : c.zones]), keys(var.zone_ids))
  matches = [
    for c in var.capabilities : [
      for g in lookup(var.groups, c.permission, []) : g.id
      if contains(g.scopes, c.account ? local.scope.account : local.scope.zone)
    ]
  ]
  policies = [
    for i, c in var.capabilities : {
      effect            = "allow"
      permission_groups = [for id in local.matches[i] : { id = id }]
      resources = (c.account
        ? jsonencode({ ("${local.scope.account}.${var.account_id}") = "*" })
        : jsonencode({ for z in c.zones : "${local.scope.zone}.${lookup(var.zone_ids, z, "")}" => "*" })
      )
    }
  ]
}

resource "cloudflare_account_token" "this" {
  account_id = var.account_id
  name       = var.name
  not_before = var.not_before
  expires_on = var.expires_on
  policies   = local.policies

  lifecycle {
    precondition {
      condition     = alltrue([for m in local.matches : length(m) == 1])
      error_message = "Each capability needs exactly one permission group of its name in its scope: ${join(", ", [for i, c in var.capabilities : c.permission if length(local.matches[i]) != 1])}."
    }
    precondition {
      condition     = length(local.unknown_zones) == 0
      error_message = "Capabilities name zones missing from config.yaml's zones: ${join(", ", sort(local.unknown_zones))}."
    }
  }
}
