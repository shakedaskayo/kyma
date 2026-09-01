variable "name" {
  type        = string
  description = "Product name, e.g. \"Pensieve Cloud — Pro\"."
}

variable "unit_label" {
  type        = string
  description = "Unit shown on invoices, e.g. \"GB ingested\"."
  default     = null
}

variable "currency" {
  type        = string
  description = "ISO currency for all prices on this product."
  default     = "usd"
}

variable "metered_prices" {
  type = list(object({
    nickname            = string
    unit_amount_decimal = number
    meter               = optional(string)
  }))
  description = "One metered price per billing meter (ingest/storage/query)."
  default     = []
}
