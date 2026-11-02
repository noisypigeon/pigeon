variable "name" {
  type = string
}

variable "capabilities" {
  type = list(object({
    permission = string
    zones      = optional(list(string), [])
    account    = optional(bool, false)
  }))
}

variable "zone_ids" {
  type = map(string)
}

variable "groups" {
  type = map(list(object({
    id     = string
    scopes = list(string)
  })))
}

variable "account_id" {
  type = string
}

variable "not_before" {
  type = string
}

variable "expires_on" {
  type = string
}
