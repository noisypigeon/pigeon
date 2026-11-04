variable "name" {
  type = string
}

variable "application_id" {
  type = string
}

variable "rules" {
  type = list(object({
    permission_sets = list(string)
    projects        = optional(list(string), [])
    organization    = optional(bool, false)
  }))
}

variable "project_ids" {
  type = map(string)
}

variable "organization_id" {
  type      = string
  sensitive = true
}
