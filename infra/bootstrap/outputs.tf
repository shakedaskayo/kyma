output "state_bucket" { value = aws_s3_bucket.state.id }
output "lock_table" { value = aws_dynamodb_table.lock.name }
output "ci_role_arn" { value = aws_iam_role.ci.arn }
