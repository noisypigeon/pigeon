variable "zone_id" {
  type = string
}

variable "name" {
  type = string
}

variable "type" {
  type = string
}

variable "content" {
  type = string
}

variable "ttl" {
  type    = number
  default = 1
}

variable "priority" {
  type    = number
  default = null
}

variable "proxied" {
  type    = bool
  default = false
}
