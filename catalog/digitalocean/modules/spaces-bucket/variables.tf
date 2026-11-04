variable "key" {
  type = string
}

variable "suffix" {
  type      = string
  sensitive = true
}

variable "region" {
  type = string
}

variable "versioning" {
  type     = bool
  default  = true
  nullable = false
}
