variable "name" {
  type        = string
  description = "Resource name prefix."
}

variable "subnet_ids" {
  type        = list(string)
  description = "Private subnet ids for the DB subnet group (>= 2 AZs)."
}

variable "vpc_id" {
  type        = string
  description = "VPC the instance lives in."
}

variable "vpc_cidr" {
  type        = string
  description = "VPC CIDR allowed to reach Postgres (the engine runs inside it; the instance is not publicly accessible)."
}

variable "password" {
  type        = string
  description = "Master password. Supplied via TF_VAR_*/secret, never committed."
  sensitive   = true
}

variable "db_name" {
  type        = string
  description = "Initial database name."
  default     = "pensieve"
}

variable "username" {
  type        = string
  description = "Master username."
  default     = "pensieve"
}

variable "instance_class" {
  type        = string
  description = "RDS instance class."
  default     = "db.t4g.micro"
}

variable "allocated_storage" {
  type        = number
  description = "Allocated storage (GiB)."
  default     = 20
}

variable "engine_version" {
  type        = string
  description = "Postgres engine version."
  default     = "16"
}
