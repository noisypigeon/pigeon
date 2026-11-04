output "id" {
  value     = digitalocean_spaces_bucket.this.id
  sensitive = true
}

output "name" {
  value     = digitalocean_spaces_bucket.this.name
  sensitive = true
}

output "urn" {
  value     = digitalocean_spaces_bucket.this.urn
  sensitive = true
}
