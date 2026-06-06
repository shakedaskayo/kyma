variable "bucket_name" {
  type        = string
  description = "Globally-unique bucket name."
}

variable "project_name" {
  type        = string
  description = "Used in tags."
}

variable "noncurrent_expiration_days" {
  type        = number
  description = "Days before noncurrent (versioned) object versions expire."
  default     = 30
}
