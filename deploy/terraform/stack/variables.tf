variable "project_name" {
  type        = string
  description = "Name prefix for every resource (cluster, bucket, SSM params)."
  default     = "kyma"
}

variable "aws_region" {
  type        = string
  description = "AWS region for the engine, S3 bucket, and SSM parameters."
  default     = "us-east-1"
}

# ---------------------------------------------------------------------------
# Supabase (catalog Postgres + Auth)
# ---------------------------------------------------------------------------

# NOTE: defaulted-empty + validated (not `required`) so the Pulumi
# terraform-module bridge can infer this module's schema — terraform >=1.15
# rejects no-args module calls at init for truly required variables.
variable "supabase_org_id" {
  type        = string
  description = "Supabase organization id that will own the project (see https://supabase.com/dashboard/org)."
  default     = ""

  validation {
    condition     = length(var.supabase_org_id) > 0
    error_message = "supabase_org_id is required."
  }
}

variable "supabase_db_password" {
  type        = string
  description = "Database password for the Supabase project. Pass via TF_VAR_supabase_db_password; never commit."
  sensitive   = true
  default     = ""

  validation {
    condition     = length(var.supabase_db_password) > 0
    error_message = "supabase_db_password is required."
  }
}

variable "supabase_region" {
  type        = string
  description = "Supabase region (AWS-aligned slug). Keep it equal to aws_region for lowest catalog latency."
  default     = "us-east-1"
}

variable "supabase_instance_size" {
  type        = string
  description = "Supabase compute add-on size; null = project default (Micro)."
  default     = null
}

# ---------------------------------------------------------------------------
# Extents storage
# ---------------------------------------------------------------------------

variable "storage_backend" {
  type        = string
  description = "Where columnar extents live: `supabase` (default — Supabase Storage via its S3 protocol, everything stateful in one place) or `s3` (native AWS S3 bucket, fully automated, lowest latency from Fargate)."
  default     = "supabase"

  validation {
    condition     = contains(["supabase", "s3"], var.storage_backend)
    error_message = "storage_backend must be \"supabase\" or \"s3\"."
  }
}

variable "storage_bucket" {
  type        = string
  description = "Bucket name for extents (created automatically for s3; create it in Supabase Storage for the supabase backend — `kyma deploy` does this for you)."
  default     = "kyma"
}

# Supabase has no API to mint Storage S3 keys — create them in the dashboard
# (Project Settings → Storage → S3 access keys) and pass them here. The
# `kyma deploy` wizard walks you through it. Leave empty when storage_backend
# is "s3".
variable "supabase_s3_access_key_id" {
  type        = string
  description = "Supabase Storage S3 access key id (supabase backend only)."
  default     = ""
}

variable "supabase_s3_secret_access_key" {
  type        = string
  description = "Supabase Storage S3 secret access key (supabase backend only)."
  sensitive   = true
  default     = ""
}

# ---------------------------------------------------------------------------
# Auth policy
# ---------------------------------------------------------------------------

variable "admin_emails" {
  type        = list(string)
  description = "Supabase-authenticated emails granted the kyma admin role."
  default     = []
}

variable "allowed_email_domains" {
  type        = list(string)
  description = "When non-empty, only these email domains may sign in (guards open Supabase signup)."
  default     = []
}

variable "oauth_providers" {
  type        = list(string)
  description = "OAuth providers to offer on the login page (must also be enabled in the Supabase dashboard), e.g. [\"google\", \"github\"]."
  default     = []
}

# ---------------------------------------------------------------------------
# Domain / TLS (optional but recommended)
# ---------------------------------------------------------------------------

variable "domain" {
  type        = string
  description = "Custom domain for the engine (e.g. kyma.example.com). Empty = plain HTTP on the ALB DNS name (demo-grade)."
  default     = ""
}

variable "route53_zone_id" {
  type        = string
  description = "Route53 hosted zone id for `domain`. When set, ACM validation and the A-record are fully automated. Empty = add the printed ACM validation CNAME manually while `apply` waits."
  default     = ""
}

# ---------------------------------------------------------------------------
# Engine sizing / image
# ---------------------------------------------------------------------------

variable "image_repo" {
  type        = string
  description = "Engine container image repository."
  default     = "ghcr.io/shakedaskayo/kyma-engine"
}

variable "image_tag" {
  type        = string
  description = "Engine image tag. Pin a release (vX.Y.Z); `kyma deploy` injects the tag matching the CLI version."
  default     = "latest"
}

variable "task_cpu" {
  type        = number
  description = "Fargate task CPU units (1024 = 1 vCPU)."
  default     = 512
}

variable "task_memory" {
  type        = number
  description = "Fargate task memory (MiB)."
  default     = 1024
}

variable "desired_count" {
  type        = number
  description = "Number of engine tasks. Keep 1 — the engine is single-writer per catalog."
  default     = 1
}
