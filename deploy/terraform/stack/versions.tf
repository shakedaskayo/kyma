# Provider-FREE module: provider configuration lives with the caller (the
# thin root in ../, or Pulumi's terraform-module bridge). Keep it that way —
# provider blocks in here would reference variables and break consumers that
# instantiate this directory as a child module.
terraform {
  required_version = ">= 1.9.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.60"
    }
    supabase = {
      source  = "supabase/supabase"
      version = "~> 1.9"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
  }
}
