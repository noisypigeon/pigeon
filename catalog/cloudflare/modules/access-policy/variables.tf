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

# One service token's ID, as `$module.<tokens>[<name>].id` gives it.
variable "service_token_id" {
  type    = string
  default = null
}

# Whether the account's Cloudflare members are let in.
variable "account_members" {
  type     = bool
  default  = false
  nullable = false
}

# Whether the policy bypasses Access for everyone, for a path its Worker authenticates itself; see
# .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md.
variable "bypass" {
  type     = bool
  default  = false
  nullable = false
}
