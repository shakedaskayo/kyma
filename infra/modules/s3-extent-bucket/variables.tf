variable "bucket_name" {
  type        = string
  description = "Globally-unique bucket name, e.g. pensieve-dev-extents."
}

variable "env" {
  type        = string
  description = "Environment name used in tags (dev/staging/prod)."
}

variable "noncurrent_expiration_days" {
  type        = number
  description = "Days before noncurrent (versioned) object versions expire."
  default     = 30
}
