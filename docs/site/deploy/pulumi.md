---
title: Deploy with Pulumi
description: Deploy kyma with Pulumi via the terraform-module bridge — same stack, Pulumi-native state and config.
---

# Deploy with Pulumi

The Terraform stack is the single source of truth; Pulumi consumes it
through the official
[terraform-module bridge](https://www.pulumi.com/docs/iac/using-pulumi/extending-pulumi/use-terraform-module/).
Identical resources, Pulumi-native state/config/secrets.

::: tip
The bridge is maintained by Pulumi but younger than core providers.
Terraform remains the primary supported path — the same stack applies
cleanly with plain Terraform if you hit a bridge issue.
:::

## Setup

```sh
cd deploy/pulumi/typescript
npm install
pulumi login --local          # or Pulumi Cloud
pulumi stack init prod

# Wrap the provider-free stack module (NOT ../../terraform, whose
# provider blocks confuse the bridge):
pulumi package add terraform-module ../../terraform/stack kymaengine
```

If `package add` prints a pnpm `pkg set` error *after* "Successfully
generated an SDK", the SDK is fine — link it manually:

```sh
npm pkg set 'dependencies.@pulumi/kymaengine=file:sdks/kymaengine'
npm install
```

## Configure

```sh
export SUPABASE_ACCESS_TOKEN=sbp_…
export AWS_REGION=us-east-1    # match awsRegion — the bridged module
                               # inherits the ambient provider region

pulumi config set supabaseOrgId <org-id>
pulumi config set --secret supabaseDbPassword "$(openssl rand -base64 24)"
pulumi config set adminEmails '["you@company.com"]'
pulumi config set allowedEmailDomains '["company.com"]'
# optional: domain, route53ZoneId, imageTag, oauthProviders …
```

## Deploy

```sh
pulumi up
pulumi stack output engineUrl
```

Teardown: `pulumi destroy`.

`kyma deploy init --tool pulumi` automates all of the above inside a
[workspace](./cli#workspaces).
