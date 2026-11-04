output "id" {
  value     = one(local.ids)
  sensitive = true

  precondition {
    condition     = length(local.ids) == 1
    error_message = "The token must see exactly one account: the pigeon account."
  }
}
