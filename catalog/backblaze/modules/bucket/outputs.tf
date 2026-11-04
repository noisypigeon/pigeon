output "id" {
  value     = b2_bucket.this.bucket_id
  sensitive = true
}

output "name" {
  value     = b2_bucket.this.bucket_name
  sensitive = true
}
