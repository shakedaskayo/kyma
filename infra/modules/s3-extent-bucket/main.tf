# tfsec:ignore:aws-s3-enable-bucket-logging access logging needs a separate log-target bucket; tracked for C5 hardening.
resource "aws_s3_bucket" "this" {
  bucket = var.bucket_name
  tags   = { Project = "kyma-cloud", Env = var.env, ManagedBy = "terraform" }
}

resource "aws_s3_bucket_versioning" "this" {
  bucket = aws_s3_bucket.this.id
  versioning_configuration { status = "Enabled" }
}

# tfsec:ignore:aws-s3-encryption-customer-key SSE-KMS with the AWS-managed key; CMK migration is part of the C5 encryption-at-rest audit.
resource "aws_s3_bucket_server_side_encryption_configuration" "this" {
  bucket = aws_s3_bucket.this.id
  rule {
    apply_server_side_encryption_by_default { sse_algorithm = "aws:kms" }
  }
}

resource "aws_s3_bucket_public_access_block" "this" {
  bucket                  = aws_s3_bucket.this.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_lifecycle_configuration" "this" {
  bucket = aws_s3_bucket.this.id
  rule {
    id     = "expire-noncurrent"
    status = "Enabled"
    filter {}
    noncurrent_version_expiration { noncurrent_days = var.noncurrent_expiration_days }
  }
}
