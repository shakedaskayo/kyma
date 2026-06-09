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
# Backend selectors (the orthogonal axes the wizard chooses interactively)
# ---------------------------------------------------------------------------

variable "compute_backend" {
  type        = string
  description = "Where the engine runs: `fargate` (ECS) or `eks` (the EKS module provisions the cluster; the CLI then installs the Helm chart). The `helm`/`local` targets do not use this Terraform stack."
  default     = "fargate"

  validation {
    condition     = contains(["fargate", "eks"], var.compute_backend)
    error_message = "compute_backend must be \"fargate\" or \"eks\"."
  }
}

variable "database_backend" {
  type        = string
  description = "Catalog Postgres provider: `supabase` (provisioned), `rds` (provisioned), or `external` (bring-your-own URL via database_url)."
  default     = "supabase"

  validation {
    condition     = contains(["supabase", "rds", "external"], var.database_backend)
    error_message = "database_backend must be \"supabase\", \"rds\", or \"external\"."
  }
}

variable "auth_backend" {
  type        = string
  description = "Sign-in backend: `supabase`, `token` (minted admin token via admin_token), or `oidc`."
  default     = "supabase"

  validation {
    condition     = contains(["supabase", "token", "oidc"], var.auth_backend)
    error_message = "auth_backend must be \"supabase\", \"token\", or \"oidc\"."
  }
}

variable "database_url" {
  type        = string
  description = "Postgres connection string for the catalog (external database backend). Pass via TF_VAR_database_url; never commit."
  sensitive   = true
  default     = ""

  validation {
    condition     = var.database_backend != "external" || length(var.database_url) > 0
    error_message = "database_url is required when database_backend = \"external\"."
  }
}

# ---------------------------------------------------------------------------
# Supabase (catalog Postgres + Auth + optional Storage)
# ---------------------------------------------------------------------------

# NOTE: defaulted-empty + conditionally validated (not `required`) so the
# Pulumi terraform-module bridge can infer this module's schema.
variable "supabase_org_id" {
  type        = string
  description = "Supabase organization id that will own the project (see https://supabase.com/dashboard/org)."
  default     = ""

  validation {
    condition     = (var.database_backend != "supabase" && var.storage_backend != "supabase") || length(var.supabase_org_id) > 0
    error_message = "supabase_org_id is required when database_backend or storage_backend = \"supabase\"."
  }
}

variable "supabase_db_password" {
  type        = string
  description = "Database password for the Supabase project. Pass via TF_VAR_supabase_db_password; never commit."
  sensitive   = true
  default     = ""

  validation {
    condition     = (var.database_backend != "supabase" && var.storage_backend != "supabase") || length(var.supabase_db_password) > 0
    error_message = "supabase_db_password is required when database_backend or storage_backend = \"supabase\"."
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

variable "rds_instance_class" {
  type        = string
  description = "RDS instance class for the catalog (rds backend)."
  default     = "db.t4g.micro"
}

# ---------------------------------------------------------------------------
# Extents storage
# ---------------------------------------------------------------------------

variable "storage_backend" {
  type        = string
  description = "Where columnar extents live: `supabase` (Supabase Storage via its S3 protocol), `s3` (native AWS S3 bucket, keyless via task/pod role), or `external` (BYO S3-compatible endpoint + keys)."
  default     = "supabase"

  validation {
    condition     = contains(["supabase", "s3", "external"], var.storage_backend)
    error_message = "storage_backend must be \"supabase\", \"s3\", or \"external\"."
  }
}

variable "storage_bucket" {
  type        = string
  description = "Bucket name for extents (created automatically for s3; for supabase/external it names the existing bucket)."
  default     = "kyma"
}

variable "supabase_s3_access_key_id" {
  type        = string
  description = "Supabase Storage S3 access key id (supabase storage backend only)."
  default     = ""
}

variable "supabase_s3_secret_access_key" {
  type        = string
  description = "Supabase Storage S3 secret access key (supabase storage backend only)."
  sensitive   = true
  default     = ""
}

variable "storage_endpoint" {
  type        = string
  description = "S3-compatible endpoint URL (external storage), e.g. https://<minio-host>:9000 or an R2 endpoint."
  default     = ""
}

variable "storage_region" {
  type        = string
  description = "Region for the external S3-compatible store (default us-east-1 if empty)."
  default     = ""
}

variable "storage_path_style" {
  type        = bool
  description = "Path-style addressing for the external store (MinIO/most S3-compat default true; AWS false)."
  default     = true
}

variable "storage_access_key" {
  type        = string
  description = "Access key id for the external S3-compatible store."
  default     = ""
}

variable "storage_secret" {
  type        = string
  description = "Secret access key for the external S3-compatible store."
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

variable "admin_token" {
  type        = string
  description = "Admin API token minted by the wizard (KYMA_AUTH_TOKENS=<token>:admin). Pass via TF_VAR_admin_token; never commit."
  sensitive   = true
  default     = ""

  validation {
    condition     = var.auth_backend != "token" || length(var.admin_token) > 0
    error_message = "admin_token is required when auth_backend = \"token\"."
  }
}

variable "oidc_issuer" {
  type        = string
  description = "OIDC issuer URL (auth_backend = oidc)."
  default     = ""

  validation {
    condition     = var.auth_backend != "oidc" || length(var.oidc_issuer) > 0
    error_message = "oidc_issuer is required when auth_backend = \"oidc\"."
  }
}

variable "oidc_client_id" {
  type        = string
  description = "OIDC client id (auth_backend = oidc)."
  default     = ""
}

# ---------------------------------------------------------------------------
# Domain / TLS (optional but recommended)
# ---------------------------------------------------------------------------

variable "domain" {
  type        = string
  description = "Custom domain for the engine (e.g. kyma.example.com). Empty = plain HTTP on the ALB DNS name (demo-grade). Fargate only."
  default     = ""
}

variable "route53_zone_id" {
  type        = string
  description = "Route53 hosted zone id for `domain`. When set, ACM validation and the A-record are fully automated."
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

# ---------------------------------------------------------------------------
# EKS sizing (compute_backend = "eks")
# ---------------------------------------------------------------------------

variable "eks_kubernetes_version" {
  type        = string
  description = "EKS control-plane Kubernetes version."
  default     = "1.31"
}

variable "eks_instance_types" {
  type        = list(string)
  description = "EKS managed node group instance types."
  default     = ["t3.medium"]
}

variable "eks_desired_size" {
  type        = number
  description = "EKS node group desired size."
  default     = 2
}
