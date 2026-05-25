# module: iam-scoped-role

The role the **gateway** assumes to read/write tenant extents, plus the
session-policy pattern that confines each request to one tenant's prefix.

The role itself grants broad access to the whole extent bucket. The gateway
never uses that broad access directly: on every request it calls
`AssumeRole(RoleArn=role_arn, Policy=<session policy>)`, passing a **session
policy** that pins the prefix to the authenticated tenant:

```json
{ "Version": "2012-10-17", "Statement": [
  { "Effect": "Allow", "Action": ["s3:GetObject","s3:PutObject","s3:DeleteObject"],
    "Resource": "arn:aws:s3:::kyma-prod-extents/${tenant_id}/*" },
  { "Effect": "Allow", "Action": "s3:ListBucket",
    "Resource": "arn:aws:s3:::kyma-prod-extents",
    "Condition": { "StringLike": { "s3:prefix": "${tenant_id}/*" } } } ]}
```

The effective permission is the **intersection** of the role policy and the
session policy, so tenant A's session can never touch tenant B's prefix — even
a buggy or compromised engine path holding that credential is confined. This is
the storage half of pooled isolation (the gateway adds quota/rate limits; the
engine adds A4 tenant-segmented paths as defense in depth).

| Input | Purpose |
|---|---|
| `env` | role name suffix (`kyma-<env>-tenant-data`) |
| `bucket_arn` | extent bucket the role can access (from `s3-extent-bucket`) |
| `gateway_principal_arn` | the only principal allowed to assume this role |

| Output | Purpose |
|---|---|
| `role_arn` | passed to the gateway; it assumes this with a per-tenant session policy |
