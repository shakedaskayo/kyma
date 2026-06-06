variable "name" {
  type        = string
  description = "Resource name prefix."
}

variable "domain" {
  type        = string
  description = "Custom domain for the engine; empty = HTTP-only on the ALB DNS name."
  default     = ""
}

variable "route53_zone_id" {
  type        = string
  description = "Route53 hosted zone for `domain` (empty = manual DNS validation)."
  default     = ""
}
