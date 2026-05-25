variable "aws_region" {
  type    = string
  default = "us-east-1"
}

variable "state_bucket_name" {
  type        = string
  description = "Globally-unique S3 bucket for Terraform remote state."
  default     = "kyma-cloud-tfstate"
}

variable "lock_table_name" {
  type    = string
  default = "kyma-cloud-tflock"
}

variable "github_org" {
  type        = string
  description = "GitHub org/owner that hosts the kyma repo."
}

variable "github_repo" {
  type    = string
  default = "kyma"
}
