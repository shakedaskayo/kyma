# Secrets as SSM SecureString parameters (free standard tier — Secrets
# Manager would add ~$0.40/secret/mo for no benefit here). ECS injects them
# into the task via `secrets[].valueFrom`, so plaintext never appears in the
# task definition.

# Keys (env-var names) are not sensitive — only the values are. for_each
# can't iterate a sensitive map, so unwrap the keys and index back in.
resource "aws_ssm_parameter" "this" {
  for_each = nonsensitive(toset(keys(var.parameters)))

  name  = "/${var.name}/${each.key}"
  type  = "SecureString"
  value = var.parameters[each.key]
  tags  = { Project = var.name, ManagedBy = "terraform" }
}
