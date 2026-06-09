output "cluster_name" {
  value       = aws_eks_cluster.this.name
  description = "EKS cluster name (for `aws eks update-kubeconfig`)."
}

output "cluster_endpoint" {
  value       = aws_eks_cluster.this.endpoint
  description = "EKS API server endpoint."
}

output "irsa_role_arn" {
  value       = aws_iam_role.irsa.arn
  description = "IAM role ARN to annotate the engine service account (eks.amazonaws.com/role-arn) for keyless S3."
}

output "oidc_provider_arn" {
  value       = aws_iam_openid_connect_provider.this.arn
  description = "Cluster IAM OIDC provider ARN."
}
