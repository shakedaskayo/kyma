resource "stripe_product" "this" {
  name       = var.name
  unit_label = var.unit_label
}

resource "stripe_price" "tier" {
  for_each = { for p in var.metered_prices : p.nickname => p }

  product             = stripe_product.this.id
  currency            = var.currency
  billing_scheme      = "per_unit"
  nickname            = each.value.nickname
  unit_amount_decimal = each.value.unit_amount_decimal

  recurring {
    interval   = "month"
    usage_type = "metered"
    meter      = each.value.meter
  }
}
