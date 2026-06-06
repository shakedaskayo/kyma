variable "name" {
  type        = string
  description = "Prefix for parameter names (/<name>/<KEY>)."
}

variable "parameters" {
  type        = map(string)
  description = "Map of env-var name → secret value."
  sensitive   = true
}
