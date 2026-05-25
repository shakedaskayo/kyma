variable "aws_region" {
  type    = string
  default = "us-east-1"
}

variable "supabase_org_id" {
  type        = string
  description = "Supabase organization id that owns the dev projects."
}

variable "supabase_region" {
  type    = string
  default = "us-east-1"
}

variable "supabase_db_pass" {
  type        = string
  description = "DB password for the dev Supabase projects (TF_VAR_supabase_db_pass)."
  sensitive   = true
}

variable "gateway_principal_arn" {
  type        = string
  description = "ARN allowed to assume the tenant-data role. TODO(C2): set to the real gateway role ARN once it exists."
  default     = "arn:aws:iam::000000000000:role/TODO-c2-gateway"
}
