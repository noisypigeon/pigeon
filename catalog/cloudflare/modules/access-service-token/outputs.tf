output "id" {
  value = cloudflare_zero_trust_access_service_token.this.id
}

output "client_id" {
  value = cloudflare_zero_trust_access_service_token.this.client_id
}

output "client_secret" {
  value     = cloudflare_zero_trust_access_service_token.this.client_secret
  sensitive = true
}
