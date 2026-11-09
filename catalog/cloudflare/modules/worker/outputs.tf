output "id" {
  value = local.id

  precondition {
    condition     = local.id != null
    error_message = "the account has no Worker by this name"
  }
}
