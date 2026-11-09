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

variable "secret_version" {
  type    = number
  default = 1
}

variable "previous_secret_expires" {
  type = string
}
