terraform {
  required_version = ">= 1.9.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.60"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
  }
  # NOTE: backend is intentionally absent. Bootstrap starts on LOCAL state,
  # then migrates to the S3 backend it creates (see README). Do not add a
  # backend block until Task 6.
}

provider "aws" {
  region = var.aws_region
  default_tags {
    tags = {
      Project   = "kyma-cloud"
      Env       = "bootstrap"
      ManagedBy = "terraform"
    }
  }
}
