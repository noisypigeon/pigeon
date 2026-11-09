variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "domain" {
  type    = string
  default = null
}

# The Worker it protects, by ID (from `cloudflare/worker`), as Worker destinations.
variable "worker_id" {
  type    = string
  default = null
}

# The Worker's URLs it protects: production (`worker`), and preview (`preview_worker`) if listed.
variable "worker_types" {
  type     = list(string)
  default  = ["worker"]
  nullable = false
}

# More policy IDs, keyed by any name, after `policy_ids`.
variable "policy_id_map" {
  type     = map(string)
  default  = {}
  nullable = false
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
