variable "name" {
  type        = string
  description = "Resource name prefix."
}

variable "aws_region" {
  type        = string
  description = "Region (for the awslogs driver)."
}

variable "image" {
  type        = string
  description = "Full engine image reference (repo:tag)."
}

variable "task_cpu" {
  type    = number
  default = 512
}

variable "task_memory" {
  type    = number
  default = 1024
}

variable "desired_count" {
  type    = number
  default = 1
}

variable "subnet_ids" {
  type        = list(string)
  description = "Public subnets for the tasks."
}

variable "service_sg_id" {
  type        = string
  description = "Security group for the engine tasks."
}

variable "target_group" {
  type        = string
  description = "ALB target group ARN."
}

variable "s3_bucket_arn" {
  type        = string
  description = "Extent bucket ARN (task-role S3 policy)."
}

variable "ssm_param_arns" {
  type        = map(string)
  description = "Env-var name → SSM parameter ARN readable by the execution role."
}

variable "environment" {
  type        = map(string)
  description = "Plain environment variables for the engine container."
}

variable "secrets" {
  type        = map(string)
  description = "Env-var name → SSM parameter ARN injected as ECS secrets."
}
