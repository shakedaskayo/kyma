# kyma self-hosted production deployment: AWS Fargate engine + S3 extents +
# Supabase (catalog Postgres + Auth).
#
# Cost-floor choices: no NAT gateway (public subnets + public task IP), SSM
# Parameter Store over Secrets Manager, single Fargate task. ~ALB + task +
# S3 + Supabase tier.

locals {
  name       = var.project_name
  engine_url = var.domain != "" ? "https://${var.domain}" : "http://${module.network.alb_dns_name}"
}

# Globally-unique-ish suffix for the S3 bucket name.
resource "random_id" "suffix" {
  byte_length = 3
}

# Encrypts connector credentials at rest inside kyma.
resource "random_password" "secret_key" {
  length  = 48
  special = false
}

# Fallback admin seeded on first boot (primary sign-in is Supabase Auth).
resource "random_password" "admin_password" {
  length  = 24
  special = false
}

module "supabase" {
  source = "./modules/supabase"

  name          = "${local.name}-catalog"
  org_id        = var.supabase_org_id
  region        = var.supabase_region
  db_pass       = var.supabase_db_password
  instance_size = var.supabase_instance_size
}

module "storage" {
  source = "./modules/storage"

  bucket_name  = "${local.name}-extents-${random_id.suffix.hex}"
  project_name = local.name
}

module "network" {
  source = "./modules/network"

  name            = local.name
  domain          = var.domain
  route53_zone_id = var.route53_zone_id
}

module "secrets" {
  source = "./modules/secrets"

  name = local.name
  parameters = {
    KYMA_CATALOG_URL    = module.supabase.database_url
    KYMA_SECRET_KEY     = random_password.secret_key.result
    KYMA_ADMIN_USER     = "admin"
    KYMA_ADMIN_PASSWORD = random_password.admin_password.result
  }
}

module "engine" {
  source = "./modules/ecs-service"

  name           = local.name
  aws_region     = var.aws_region
  image          = "${var.image_repo}:${var.image_tag}"
  task_cpu       = var.task_cpu
  task_memory    = var.task_memory
  desired_count  = var.desired_count
  subnet_ids     = module.network.public_subnet_ids
  service_sg_id  = module.network.service_security_group_id
  target_group   = module.network.target_group_arn
  s3_bucket_arn  = module.storage.bucket_arn
  ssm_param_arns = module.secrets.parameter_arns

  environment = {
    KYMA_HTTP_ADDR = "0.0.0.0:8080"
    # gRPC (Arrow Flight) + OTLP-gRPC need an NLB; disabled in this stack.
    # OTLP/HTTP ingest works through the ALB at /v1/ingest.
    KYMA_GRPC_ADDR             = "off"
    KYMA_OTLP_ADDR             = "off"
    KYMA_S3_BUCKET             = module.storage.bucket_name
    KYMA_S3_REGION             = var.aws_region
    KYMA_S3_PATH_STYLE         = "false"
    KYMA_S3_ALLOW_HTTP         = "false"
    KYMA_AUTH_BACKEND          = "supabase"
    KYMA_SUPABASE_URL          = module.supabase.project_url
    KYMA_SUPABASE_ANON_KEY     = module.supabase.anon_key
    KYMA_SUPABASE_PROVIDERS    = join(",", var.oauth_providers)
    KYMA_ADMIN_EMAILS          = join(",", var.admin_emails)
    KYMA_ALLOWED_EMAIL_DOMAINS = join(",", var.allowed_email_domains)
    KYMA_OAUTH_REDIRECT_BASE   = local.engine_url
  }

  secrets = module.secrets.parameter_arns
}
