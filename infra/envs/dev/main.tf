provider "aws" {
  region = var.aws_region
  default_tags {
    tags = { Project = "kyma-cloud", Env = "dev", ManagedBy = "terraform" }
  }
}

# Credentials come from env in CI: SUPABASE_ACCESS_TOKEN / STRIPE_API_KEY.
provider "supabase" {}
provider "stripe" {}

# --- Storage: one extent bucket; pooled tenants are prefixes within it ---
module "extents" {
  source      = "../../modules/s3-extent-bucket"
  bucket_name = "kyma-dev-extents"
  env         = "dev"
}

# --- Supabase A: control-plane DB (orgs/projects/api_keys/usage/billing) ---
module "control_plane_db" {
  source  = "../../modules/supabase-project"
  name    = "kyma-dev-control-plane"
  org_id  = var.supabase_org_id
  region  = var.supabase_region
  db_pass = var.supabase_db_pass
}

# --- Supabase B: engine catalog (manifests/stats/CAS) — separate blast radius ---
module "catalog" {
  source  = "../../modules/supabase-project"
  name    = "kyma-dev-catalog"
  org_id  = var.supabase_org_id
  region  = var.supabase_region
  db_pass = var.supabase_db_pass
}

# --- Per-tenant STS scoping role (gateway assumes with a session policy) ---
module "tenant_role" {
  source                = "../../modules/iam-scoped-role"
  env                   = "dev"
  bucket_arn            = module.extents.bucket_arn
  gateway_principal_arn = var.gateway_principal_arn
}

# --- Billing: one product per tier. Metered prices + meters land in C4. ---
module "billing_free" {
  source         = "../../modules/stripe-product"
  name           = "Kyma Cloud — Free"
  unit_label     = "GB"
  metered_prices = []
}

module "billing_pro" {
  source         = "../../modules/stripe-product"
  name           = "Kyma Cloud — Pro"
  unit_label     = "GB"
  metered_prices = [] # TODO(C4): ingest/storage/query prices tied to billing meters
}

module "billing_enterprise" {
  source         = "../../modules/stripe-product"
  name           = "Kyma Cloud — Enterprise"
  unit_label     = "GB"
  metered_prices = [] # TODO(C4): ingest/storage/query prices tied to billing meters
}

# --- Railway services: deferred (engine image + deploy come with C1/C2/C3) ---
# module "engine" {
#   source     = "../../modules/railway-service"   # TODO(C1)
#   project_id = var.railway_project_id
#   name       = "kyma-dev-engine"
#   image      = var.engine_image
#   env_vars   = { KYMA_S3_BUCKET = module.extents.bucket_name, KYMA_S3_REGION = var.aws_region, ... }
# }
# module "gateway"   { source = "../../modules/railway-service" ... }  # TODO(C2)
# module "api"       { source = "../../modules/railway-service" ... }  # TODO(C2)
# module "dashboard" { source = "../../modules/railway-service" ... }  # TODO(C3)
