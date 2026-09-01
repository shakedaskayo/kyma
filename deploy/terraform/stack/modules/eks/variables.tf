variable "name" {
  type        = string
  description = "Resource name prefix (also the EKS cluster name)."
}

variable "subnet_ids" {
  type        = list(string)
  description = "Subnet ids for the cluster + node group (>= 2 AZs)."
}

variable "kubernetes_version" {
  type        = string
  description = "EKS control-plane Kubernetes version."
  default     = "1.31"
}

variable "instance_types" {
  type        = list(string)
  description = "Managed node group instance types."
  default     = ["t3.medium"]
}

variable "desired_size" {
  type    = number
  default = 2
}

variable "min_size" {
  type    = number
  default = 1
}

variable "max_size" {
  type    = number
  default = 3
}

variable "namespace" {
  type        = string
  description = "Kubernetes namespace the engine runs in (for the IRSA trust)."
  default     = "pensieve"
}

variable "service_account" {
  type        = string
  description = "Kubernetes service account name (for the IRSA trust)."
  default     = "pensieve-engine"
}

variable "s3_bucket_arn" {
  type        = string
  description = "Extent bucket ARN for the IRSA S3 policy. Empty = no S3 policy (non-S3 storage)."
  default     = ""
}
