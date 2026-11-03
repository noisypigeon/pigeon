output "id" {
  value = digitalocean_spaces_key.this.id
}

output "name" {
  value = digitalocean_spaces_key.this.name
}

output "access_key" {
  value     = digitalocean_spaces_key.this.access_key
  sensitive = true
}

output "secret_key" {
  value     = digitalocean_spaces_key.this.secret_key
  sensitive = true
}
