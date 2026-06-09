# Thin root: configures providers (see versions.tf) and instantiates the
# provider-free stack module. Pulumi users wrap ./stack directly via the
# terraform-module bridge — keep all resources in there, not here.

module "kyma" {
  source = "./stack"

  project_name = var.project_name
  aws_region   = var.aws_region

  compute_backend  = var.compute_backend
  database_backend = var.database_backend
  auth_backend     = var.auth_backend
  database_url     = var.database_url

  supabase_org_id        = var.supabase_org_id
  supabase_db_password   = var.supabase_db_password
  supabase_region        = var.supabase_region
  supabase_instance_size = var.supabase_instance_size
  rds_instance_class     = var.rds_instance_class

  storage_backend               = var.storage_backend
  storage_bucket                = var.storage_bucket
  supabase_s3_access_key_id     = var.supabase_s3_access_key_id
  supabase_s3_secret_access_key = var.supabase_s3_secret_access_key
  storage_endpoint              = var.storage_endpoint
  storage_region                = var.storage_region
  storage_path_style            = var.storage_path_style
  storage_access_key            = var.storage_access_key
  storage_secret                = var.storage_secret

  admin_emails          = var.admin_emails
  allowed_email_domains = var.allowed_email_domains
  oauth_providers       = var.oauth_providers
  admin_token           = var.admin_token
  oidc_issuer           = var.oidc_issuer
  oidc_client_id        = var.oidc_client_id

  domain          = var.domain
  route53_zone_id = var.route53_zone_id

  image_repo    = var.image_repo
  image_tag     = var.image_tag
  task_cpu      = var.task_cpu
  task_memory   = var.task_memory
  desired_count = var.desired_count

  eks_kubernetes_version = var.eks_kubernetes_version
  eks_instance_types     = var.eks_instance_types
  eks_desired_size       = var.eks_desired_size
}
