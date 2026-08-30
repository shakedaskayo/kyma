terraform {
  backend "s3" {
    bucket         = "pensieve-cloud-tfstate"
    key            = "bootstrap/bootstrap.tfstate"
    region         = "us-east-1"
    dynamodb_table = "pensieve-cloud-tflock"
    encrypt        = true
  }
}
