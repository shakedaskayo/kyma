output "service_name" {
  value       = var.name
  description = "Service name (stable handle until C1 wires real id capture)."
}

# NOTE: service_id and public_domain are produced by the real CLI/provider
# wiring in C1 (via `railway status --json` through an `external` data source).
# They are intentionally absent from this skeleton rather than faked.
