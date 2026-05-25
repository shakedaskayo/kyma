variable "project_id" {
  type        = string
  description = "Railway project id this service belongs to."
}

variable "name" {
  type        = string
  description = "Service name, e.g. kyma-dev-engine."
}

variable "image" {
  type        = string
  description = "Container image reference to deploy (e.g. ghcr.io/.../kyma-bin:tag)."
}

variable "region" {
  type        = string
  description = "Railway region (logical; co-located with us-east-1)."
  default     = "us-east-1"
}

variable "replicas" {
  type        = number
  description = "Replica count for the service."
  default     = 1
}

variable "env_vars" {
  type        = map(string)
  description = "Service environment variables (KYMA_* config; may hold secrets)."
  default     = {}
  sensitive   = true
}
