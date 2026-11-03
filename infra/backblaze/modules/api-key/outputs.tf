output "id" {
  value     = b2_application_key.this.application_key_id
  sensitive = true
}

output "name" {
  value = b2_application_key.this.key_name
}

output "value" {
  value     = b2_application_key.this.application_key
  sensitive = true
}
