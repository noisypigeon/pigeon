output "id" {
  value = cloudflare_account_token.this.id
}

output "name" {
  value = cloudflare_account_token.this.name
}

output "value" {
  value     = cloudflare_account_token.this.value
  sensitive = true
}
