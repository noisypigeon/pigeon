variable "name" {
  type = string
}

variable "capabilities" {
  type = set(string)
}

variable "buckets" {
  type = set(string)
}

variable "bucket_ids" {
  type      = map(string)
  sensitive = true
}

variable "period_days" {
  type = number
}

variable "overlap_days" {
  type = number
}

variable "not_before" {
  type = string
}

variable "expires_on" {
  type = string
}
