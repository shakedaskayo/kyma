# Pinned provider sources (confirmed C0 Task 4 — 2026-05-25)

| Provider | Source | Version | Notes |
|---|---|---|---|
| AWS | `hashicorp/aws` | `~> 5.60` | first-class |
| Supabase | `supabase/supabase` | `~> 1.9` | official (supabase org), latest 1.9.1; projects, settings |
| Stripe | `lukasaron/stripe` | `~> 3.4` | community, latest 3.4.1, ~3.2M downloads; products, prices, webhooks |
| Railway | **CLI fallback** (`null_resource` + `railway` CLI) | provider `terraform-community-providers/railway` 0.6.2 exists | see below |

## Railway: CLI fallback, not the provider (for now)

The only Railway Terraform provider, `terraform-community-providers/railway`, is
**pre-1.0 (0.6.2, ~48k downloads)** — community tier with a still-churning
resource schema. C0 only needs validatable module *skeletons*; Railway services
are not provisioned until C1. Rather than couple the foundation to a 0.x schema,
the `railway-service` module wraps the Railway CLI in a `null_resource` with a
documented `triggers` map; state tracks only the returned `service_id`. This is
the master design's sanctioned fallback ("fall back to provider-specific
config-as-code where a TF provider is thin").

**Revisit at C1:** when we do the real engine/gateway deploy and can pin against a
working schema, re-evaluate adopting `terraform-community-providers/railway`.

If a provider is the CLI fallback, the module wraps the Railway CLI in a
`null_resource` with a documented `triggers` map; state tracks only the
service id returned.
