variable "length" {
  type     = number
  default  = 6
  nullable = false
}

variable "keepers" {
  type     = map(string)
  default  = {}
  nullable = false
}
