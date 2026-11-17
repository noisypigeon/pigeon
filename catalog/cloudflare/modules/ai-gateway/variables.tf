variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "store_id" {
  type      = string
  sensitive = true
}

variable "rate_limit_per_minute" {
  type = number
}
