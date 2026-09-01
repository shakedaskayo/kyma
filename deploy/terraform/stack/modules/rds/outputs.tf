output "database_url" {
  value       = "postgresql://${var.username}:${var.password}@${aws_db_instance.this.address}:5432/${var.db_name}"
  description = "Postgres connection string for the pensieve catalog."
  sensitive   = true
}

output "endpoint" {
  value       = aws_db_instance.this.address
  description = "RDS instance hostname."
}
