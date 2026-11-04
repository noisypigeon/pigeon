output "name" {
  value     = cloudflare_r2_bucket.this.name
  sensitive = true
}
