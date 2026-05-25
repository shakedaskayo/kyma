output "product_id" {
  value       = stripe_product.this.id
  description = "Stripe product id."
}

output "price_ids" {
  value       = { for k, p in stripe_price.tier : k => p.id }
  description = "Map of price nickname -> Stripe price id."
}
