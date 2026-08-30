# Deploying pensieve with Pulumi

The Terraform stack in [`../terraform/stack`](../terraform/stack) is the
single source of truth; Pulumi consumes it through the official
[terraform-module bridge](https://www.pulumi.com/docs/iac/using-pulumi/extending-pulumi/use-terraform-module/)
— no re-implementation, identical resources, Pulumi-native state/config/secrets.

> The bridge is maintained by Pulumi but younger than core IaC providers.
> Terraform remains pensieve's primary supported path; if you hit a bridge issue,
> the same stack applies cleanly with plain Terraform.

## Setup

```sh
cd deploy/pulumi/typescript
npm install
pulumi login --local        # or Pulumi Cloud
pulumi stack init prod

# Generate the local SDK wrapping ../../terraform/stack (the provider-free
# module — NOT ../../terraform, whose provider blocks confuse the bridge):
pulumi package add terraform-module ../../terraform/stack pensieveengine
```

If `package add` prints a pnpm `pkg set` error after "Successfully generated
an SDK", the SDK is fine — link it manually:

```sh
npm pkg set 'dependencies.@pulumi/pensieveengine=file:sdks/pensieveengine'
npm install
```

## Configure

AWS credentials come from the standard chain; the Supabase provider reads
`SUPABASE_ACCESS_TOKEN` from the environment. Set `AWS_REGION` to the same
value as the `awsRegion` config — the bridged module inherits the ambient
provider region.

```sh
export SUPABASE_ACCESS_TOKEN=sbp_…   # https://supabase.com/dashboard/account/tokens
export AWS_REGION=us-east-1

pulumi config set supabaseOrgId <org-id>
pulumi config set --secret supabaseDbPassword "$(openssl rand -base64 24)"
pulumi config set adminEmails '["you@company.com"]'
pulumi config set allowedEmailDomains '["company.com"]'
# optional:
# pulumi config set domain pensieve.company.com
# pulumi config set route53ZoneId Z0123456789ABC
# pulumi config set imageTag v0.1.0
```

## Deploy

```sh
pulumi up
pulumi stack output engineUrl
```

Then `pensieve connect "$(pulumi stack output engineUrl)" --token <api-token>`
(mint the token under Settings → API tokens after signing in).

Teardown: `pulumi destroy`.
