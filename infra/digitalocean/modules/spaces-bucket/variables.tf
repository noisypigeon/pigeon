variable "prefix" {
  type      = string
  sensitive = true
}

variable "key" {
  type = string
}

variable "region" {
  type = string
}

variable "versioning" {
  type     = bool
  default  = true
  nullable = false
}
