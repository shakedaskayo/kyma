data "aws_iam_policy_document" "assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "AWS"
      identifiers = [var.gateway_principal_arn]
    }
  }
}

resource "aws_iam_role" "tenant_data" {
  name               = "kyma-${var.env}-tenant-data"
  assume_role_policy = data.aws_iam_policy_document.assume.json
}

# Broad bucket access on the ROLE; the gateway passes a SESSION POLICY at
# AssumeRole time that restricts s3:prefix to s3://<bucket>/<tenant_id>/*.
# Effective permission = intersection(role policy, session policy).
data "aws_iam_policy_document" "bucket_rw" {
  statement {
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]
    resources = ["${var.bucket_arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [var.bucket_arn]
  }
}

resource "aws_iam_role_policy" "bucket_rw" {
  role   = aws_iam_role.tenant_data.id
  policy = data.aws_iam_policy_document.bucket_rw.json
}
