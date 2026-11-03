variable "name" {
  type = string
}

variable "grants" {
  type = list(object({
    bucket     = string
    permission = string
  }))
}

variable "bucket_names" {
  type      = map(string)
  sensitive = true
}

variable "not_before" {
  type = string
}

variable "expires_on" {
  type = string
}
