variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "duration" {
  type    = string
  default = "8760h"
}
