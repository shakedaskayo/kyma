variable "env" {
  type        = string
  description = "Environment name used in the role name (dev/staging/prod)."
}

variable "bucket_arn" {
  type        = string
  description = "ARN of the extent bucket this role grants access to."
}

variable "gateway_principal_arn" {
  type        = string
  description = "ARN of the gateway's task/role allowed to assume this role."
}
