# State lives locally by default — simplest for a single operator.
#
# For teams (shared state + locking), create an S3 bucket + DynamoDB table
# once, then uncomment and `terraform init -migrate-state`:
#
# terraform {
#   backend "s3" {
#     bucket         = "<your-tf-state-bucket>"
#     key            = "kyma/deploy.tfstate"
#     region         = "us-east-1"
#     dynamodb_table = "<your-tf-lock-table>"
#     encrypt        = true
#   }
# }
