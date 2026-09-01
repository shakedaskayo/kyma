# Bootstrap (run once, by hand)

> **Operator step — not CI, not automated.** This is the only `terraform apply`
> a human runs locally. It needs AWS admin credentials for the **ops/bootstrap
> account** exported in your shell. Everything after this authenticates via
> GitHub OIDC.

## 1. First apply (local state)

```bash
cd infra/bootstrap
terraform init -backend=false
terraform apply -var github_org=<ORG>     # creates state bucket, lock, OIDC, CI role
```

Record the outputs — `state_bucket`, `lock_table`, `ci_role_arn`.

## 2. Migrate this layer's state into the bucket it just created

`backend.tf` is already committed (S3 backend pointing at `pensieve-cloud-tfstate`).
With it present:

```bash
terraform init -migrate-state    # answer "yes" to copy local state → S3
```

Expected: `Successfully configured the backend "s3"!`

## 3. Wire CI

Put `ci_role_arn` into the GitHub Actions secret `AWS_CI_ROLE_ARN` (see
`infra/README.md` → CI secrets). From here on, `envs/*` apply via OIDC; no
human runs `apply` outside this directory.

## Local validation without creds

To `validate` this layer without touching AWS (e.g. in review), ignore the
backend block:

```bash
terraform init -backend=false && terraform validate
```
