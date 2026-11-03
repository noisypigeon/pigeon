output "id" {
  value = one(digitalocean_project_resources.this[*].id)
}
