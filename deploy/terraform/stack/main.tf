# pensieve self-hosted production deployment. Pluggable along four axes:
#   compute_backend  : fargate | eks
#   database_backend : supabase | rds | external
#   storage_backend  : supabase | s3 | external
#   auth_backend     : supabase | token | oidc
# The engine is backend-agnostic; this stack wires the chosen backends. The
# `helm`/`local` targets render their own artifacts (values.yaml / local.env)
# and do not use this stack.
#
# Cost-floor choices: no NAT gateway (public subnets + public IPs), SSM
# Parameter Store over Secrets Manager, single engine task/replica.

locals {
  name         = var.project_name
  is_fargate   = var.compute_backend == "fargate"
  is_eks       = var.compute_backend == "eks"
  use_supabase = var.database_backend == "supabase" || var.storage_backend == "supabase"
  s3_native    = var.storage_backend == "s3"
  ext_storage  = var.storage_backend == "external"

  alb_dns    = module.network.alb_dns_name
  engine_url = local.is_fargate ? (var.domain != "" ? "https://${var.domain}" : "http://${local.alb_dns}") : (var.domain != "" ? "https://${var.domain}" : "")
}

# Globally-unique-ish suffix for the S3 bucket name.
resource "random_id" "suffix" {
  byte_length = 3
}

# Encrypts data source credentials at rest inside pensieve.
resource "random_password" "secret_key" {
  length  = 48
  special = false
}

# Fallback admin seeded on first boot.
resource "random_password" "admin_password" {
  length  = 24
  special = false
}

# RDS master password (rds database backend).
resource "random_password" "rds_password" {
  length  = 24
  special = false
}

# ---------------------------------------------------------------------------
# Catalog database
# ---------------------------------------------------------------------------

module "supabase" {
  source = "./modules/supabase"
  count  = local.use_supabase ? 1 : 0

  name          = "${local.name}-catalog"
  org_id        = var.supabase_org_id
  region        = var.supabase_region
  db_pass       = var.supabase_db_password
  instance_size = var.supabase_instance_size
}

module "rds" {
  source = "./modules/rds"
  count  = var.database_backend == "rds" ? 1 : 0

  name           = local.name
  subnet_ids     = module.network.private_subnet_ids
  vpc_id         = module.network.vpc_id
  vpc_cidr       = module.network.vpc_cidr
  password       = random_password.rds_password.result
  instance_class = var.rds_instance_class
}

locals {
  sb_url     = one(module.supabase[*].database_url)
  sb_project = one(module.supabase[*].project_url)
  sb_anon    = one(module.supabase[*].anon_key)
  sb_s3_ep   = one(module.supabase[*].storage_s3_endpoint)

  catalog_url = (
    var.database_backend == "supabase" ? local.sb_url :
    var.database_backend == "rds" ? one(module.rds[*].database_url) :
    var.database_url
  )
}

# ---------------------------------------------------------------------------
# Object storage
# ---------------------------------------------------------------------------

module "storage" {
  source = "./modules/storage"
  count  = local.s3_native ? 1 : 0

  bucket_name  = "${local.name}-extents-${random_id.suffix.hex}"
  project_name = local.name
}

locals {
  # Non-secret storage env (S3/Supabase/external).
  storage_environment = (
    local.s3_native ? {
      PENSIEVE_S3_BUCKET     = one(module.storage[*].bucket_name)
      PENSIEVE_S3_REGION     = var.aws_region
      PENSIEVE_S3_PATH_STYLE = "false"
      PENSIEVE_S3_ALLOW_HTTP = "false"
    } :
    local.ext_storage ? {
      PENSIEVE_S3_ENDPOINT   = var.storage_endpoint
      PENSIEVE_S3_BUCKET     = var.storage_bucket
      PENSIEVE_S3_REGION     = var.storage_region != "" ? var.storage_region : var.aws_region
      PENSIEVE_S3_PATH_STYLE = tostring(var.storage_path_style)
      PENSIEVE_S3_ALLOW_HTTP = "false"
    } :
    {
      PENSIEVE_S3_ENDPOINT   = local.sb_s3_ep
      PENSIEVE_S3_BUCKET     = var.storage_bucket
      PENSIEVE_S3_REGION     = var.supabase_region
      PENSIEVE_S3_PATH_STYLE = "true"
      PENSIEVE_S3_ALLOW_HTTP = "false"
    }
  )

  # Secret storage env (access keys). Native S3 is keyless (task/pod role).
  storage_secrets = (
    local.s3_native ? {} :
    local.ext_storage ? {
      PENSIEVE_S3_ACCESS_KEY_ID     = var.storage_access_key
      PENSIEVE_S3_SECRET_ACCESS_KEY = var.storage_secret
    } :
    {
      PENSIEVE_S3_ACCESS_KEY_ID     = var.supabase_s3_access_key_id
      PENSIEVE_S3_SECRET_ACCESS_KEY = var.supabase_s3_secret_access_key
    }
  )
}

# ---------------------------------------------------------------------------
# Auth env (supabase | token | oidc)
# ---------------------------------------------------------------------------

locals {
  auth_environment = (
    var.auth_backend == "supabase" ? {
      PENSIEVE_AUTH_BACKEND          = "supabase"
      PENSIEVE_SUPABASE_URL          = local.sb_project
      PENSIEVE_SUPABASE_ANON_KEY     = local.sb_anon
      PENSIEVE_SUPABASE_PROVIDERS    = join(",", var.oauth_providers)
      PENSIEVE_ADMIN_EMAILS          = join(",", var.admin_emails)
      PENSIEVE_ALLOWED_EMAIL_DOMAINS = join(",", var.allowed_email_domains)
      PENSIEVE_OAUTH_REDIRECT_BASE   = local.engine_url
    } :
    var.auth_backend == "oidc" ? {
      PENSIEVE_AUTH_BACKEND   = "oidc"
      PENSIEVE_OIDC_ISSUER    = var.oidc_issuer
      PENSIEVE_OIDC_CLIENT_ID = var.oidc_client_id
      PENSIEVE_ADMIN_EMAILS   = join(",", var.admin_emails)
    } :
    {
      PENSIEVE_AUTH_BACKEND = "token"
    }
  )

  auth_secrets = var.auth_backend == "token" ? {
    PENSIEVE_AUTH_TOKENS = "${var.admin_token}:admin"
  } : {}
}

# ---------------------------------------------------------------------------
# Network (always — both fargate + eks need the VPC; ALB only for fargate)
# ---------------------------------------------------------------------------

module "network" {
  source = "./modules/network"

  name            = local.name
  domain          = var.domain
  route53_zone_id = var.route53_zone_id
  create_alb      = local.is_fargate
}

# ---------------------------------------------------------------------------
# Secrets (SSM) — Fargate only; the EKS/Helm paths inject secrets via the
# Kubernetes Secret rendered into the chart by the CLI.
# ---------------------------------------------------------------------------

module "secrets" {
  source = "./modules/secrets"
  count  = local.is_fargate ? 1 : 0

  name = local.name
  parameters = merge({
    PENSIEVE_CATALOG_URL    = local.catalog_url
    PENSIEVE_SECRET_KEY     = random_password.secret_key.result
    PENSIEVE_ADMIN_USER     = "admin"
    PENSIEVE_ADMIN_PASSWORD = random_password.admin_password.result
  }, local.storage_secrets, local.auth_secrets)
}

# ---------------------------------------------------------------------------
# Compute: Fargate service OR EKS cluster (CLI installs the Helm chart on EKS)
# ---------------------------------------------------------------------------

module "engine" {
  source = "./modules/ecs-service"
  count  = local.is_fargate ? 1 : 0

  name           = local.name
  aws_region     = var.aws_region
  image          = "${var.image_repo}:${var.image_tag}"
  task_cpu       = var.task_cpu
  task_memory    = var.task_memory
  desired_count  = var.desired_count
  subnet_ids     = module.network.public_subnet_ids
  service_sg_id  = module.network.service_security_group_id
  target_group   = module.network.target_group_arn
  s3_bucket_arn  = local.s3_native ? one(module.storage[*].bucket_arn) : ""
  ssm_param_arns = module.secrets[0].parameter_arns

  environment = merge({
    PENSIEVE_HTTP_ADDR = "0.0.0.0:8080"
    # gRPC (Arrow Flight) + OTLP-gRPC need an NLB; disabled in this stack.
    # OTLP/HTTP ingest works through the ALB at /v1/ingest.
    PENSIEVE_GRPC_ADDR = "off"
    PENSIEVE_OTLP_ADDR = "off"
  }, local.auth_environment, local.storage_environment)

  secrets = module.secrets[0].parameter_arns
}

module "eks" {
  source = "./modules/eks"
  count  = local.is_eks ? 1 : 0

  name               = local.name
  subnet_ids         = module.network.public_subnet_ids
  kubernetes_version = var.eks_kubernetes_version
  instance_types     = var.eks_instance_types
  desired_size       = var.eks_desired_size
  s3_bucket_arn      = local.s3_native ? one(module.storage[*].bucket_arn) : ""
}
