terraform {
  required_version = ">= 1.9.0"
  required_providers {
    stripe = {
      source  = "lukasaron/stripe"
      version = "~> 3.4"
    }
  }
}
