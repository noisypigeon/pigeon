output "id" {
  value     = data.cloudflare_zone.this.id
  sensitive = true
}

output "name" {
  value = data.cloudflare_zone.this.name
}
