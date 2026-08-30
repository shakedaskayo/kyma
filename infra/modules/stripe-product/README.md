# module: stripe-product

One Stripe product per plan tier with a metered price per billing meter
(decision #9: ingest bytes/rows, storage GB-month, query bytes-scanned /
Flight-time). Prices are `per_unit` metered monthly-recurring, each tied to a
Stripe billing `meter`.

| Input | Purpose |
|---|---|
| `name` | product name (e.g. "Pensieve Cloud — Pro") |
| `unit_label` | invoice unit label |
| `currency` | ISO currency (default `usd`) |
| `metered_prices` | list of `{ nickname, unit_amount_decimal, meter }` |

| Output | Purpose |
|---|---|
| `product_id` | Stripe product id |
| `price_ids` | map nickname → price id (stored in control-plane DB in C4) |

`meter` references a Stripe billing meter id (created in C4 when metering lands).
Until then a tier can be defined with an empty `metered_prices` list. Webhook
wiring (subscription state → control-plane DB) is C4, not this module.
