# Thin root: configures providers (see versions.tf) and instantiates the
# provider-free stack module. Pulumi users wrap ./stack directly via the
# terraform-module bridge — keep all resources in there, not here.

module "kyma" {
  source = "./stack"

  project_name           = var.project_name
  aws_region             = var.aws_region
  supabase_org_id        = var.supabase_org_id
  supabase_db_password   = var.supabase_db_password
  supabase_region        = var.supabase_region
  supabase_instance_size = var.supabase_instance_size
  admin_emails           = var.admin_emails
  allowed_email_domains  = var.allowed_email_domains
  oauth_providers        = var.oauth_providers
  domain                 = var.domain
  route53_zone_id        = var.route53_zone_id
  image_repo             = var.image_repo
  image_tag              = var.image_tag
  task_cpu               = var.task_cpu
  task_memory            = var.task_memory
  desired_count          = var.desired_count
}
