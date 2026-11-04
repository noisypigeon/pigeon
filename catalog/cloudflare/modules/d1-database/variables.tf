variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "location" {
  type    = string
  default = "weur"
}
