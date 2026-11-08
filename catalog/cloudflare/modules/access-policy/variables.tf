variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "emails" {
  type     = list(string)
  default  = []
  nullable = false
}

# Service token IDs, keyed by any name, as a unit's `$module.<tokens>[*].id` gives them.
variable "service_token_ids" {
  type     = map(string)
  default  = {}
  nullable = false
}
