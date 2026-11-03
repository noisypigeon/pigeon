output "id" {
  value = scaleway_iam_api_key.this.id
}

output "access_key" {
  value     = scaleway_iam_api_key.this.access_key
  sensitive = true
}

output "secret_key" {
  value     = scaleway_iam_api_key.this.secret_key
  sensitive = true
}
