variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "domain" {
  type = string
}

variable "session_duration" {
  type    = string
  default = "24h"
}

variable "policy_ids" {
  type     = list(string)
  default  = []
  nullable = false
}
