variable "name" {
  type        = string
  description = "Project name, e.g. pensieve-dev-catalog or pensieve-dev-control-plane."
}

variable "org_id" {
  type        = string
  description = "Supabase organization id that owns the project."
}

variable "region" {
  type        = string
  description = "Supabase region (AWS-aligned slug, e.g. us-east-1)."
  default     = "us-east-1"
}

variable "db_pass" {
  type        = string
  description = "Database password. Supplied via TF_VAR_*/secret, never committed."
  sensitive   = true
}

variable "instance_size" {
  type        = string
  description = "Compute add-on size; null = project default."
  default     = null
}
