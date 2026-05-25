terraform {
  required_version = ">= 1.9.0"
  required_providers {
    aws      = { source = "hashicorp/aws", version = "~> 5.60" }
    supabase = { source = "supabase/supabase", version = "~> 1.9" }
    stripe   = { source = "lukasaron/stripe", version = "~> 3.4" }
    null     = { source = "hashicorp/null", version = "~> 3.2" }
  }
}
