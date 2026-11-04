variable "account_id" {
  type      = string
  sensitive = true
}

variable "name" {
  type = string
}

variable "suffix" {
  type    = string
  default = null
}

variable "jurisdiction" {
  type    = string
  default = "eu"
}

variable "storage_class" {
  type    = string
  default = "Standard"
}
