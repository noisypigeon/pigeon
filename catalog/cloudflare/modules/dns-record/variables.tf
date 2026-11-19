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
  type     = string
  default  = null
  nullable = true
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

variable "data" {
  type    = any
  default = null
}

variable "comment" {
  type    = string
  default = null
}
