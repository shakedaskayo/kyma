# CLI fallback (see ../_providers.md): the only Railway TF provider is pre-1.0,
# so a service is provisioned by wrapping the Railway CLI in a null_resource.
# `triggers` force a redeploy when any input changes; env_vars are hashed so
# secrets never land in state as plaintext.
resource "null_resource" "this" {
  triggers = {
    project_id = var.project_id
    name       = var.name
    image      = var.image
    region     = var.region
    replicas   = tostring(var.replicas)
    env_hash   = sha256(jsonencode(var.env_vars))
  }

  # TODO(C1): replace this stub with the real `railway` CLI sequence
  # (service up + `railway variables --set`) once we do the live engine deploy
  # and can capture the service id / public domain via `railway status --json`.
  provisioner "local-exec" {
    command = "echo 'railway deploy stub: service=${var.name} project=${var.project_id} image=${var.image} region=${var.region} replicas=${var.replicas}'"
  }
}
