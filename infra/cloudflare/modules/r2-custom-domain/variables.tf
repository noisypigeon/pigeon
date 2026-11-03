variable "account_id" {
  type      = string
  sensitive = true
}

variable "bucket_name" {
  type = string
}

variable "jurisdiction" {
  type    = string
  default = "eu"
}

variable "domain" {
  type = string
}

variable "zone_id" {
  type = string
}

variable "enabled" {
  type    = bool
  default = true
}

variable "min_tls" {
  type    = string
  default = "1.2"
}
