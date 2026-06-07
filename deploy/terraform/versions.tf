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

provider "aws" {
  region = var.aws_region
}

# Reads SUPABASE_ACCESS_TOKEN from the environment (or set access_token here).
provider "supabase" {}
