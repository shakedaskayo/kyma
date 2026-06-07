variable "name" {
  type        = string
  description = "Supabase project name."
}

variable "org_id" {
  type        = string
  description = "Supabase organization id that owns the project."
}

variable "region" {
  type        = string
  description = "Supabase region (AWS-aligned slug)."
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
