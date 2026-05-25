terraform {
  backend "s3" {
    bucket         = "kyma-cloud-tfstate"
    key            = "bootstrap/bootstrap.tfstate"
    region         = "us-east-1"
    dynamodb_table = "kyma-cloud-tflock"
    encrypt        = true
  }
}
