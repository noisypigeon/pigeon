locals {
  result = data.cloudflare_account_api_token_permission_groups_list.this.result
}

output "id" {
  value = sha1(join(",", sort([for g in local.result : g.id])))
}

output "groups" {
  value = { for g in local.result : g.name => { id = g.id, scopes = g.scopes }... }
}
