# AWS RDS Postgres as the kyma catalog. Private (not publicly accessible),
# encrypted, with automated backups. Reachable only from inside the VPC where
# the engine runs (Fargate task / EKS node). The engine self-migrates the
# schema on first connect — no bootstrap job.

resource "aws_db_subnet_group" "this" {
  name       = "${var.name}-catalog"
  subnet_ids = var.subnet_ids
  tags       = { Project = var.name, ManagedBy = "terraform" }
}

resource "aws_security_group" "this" {
  name        = "${var.name}-catalog-db"
  description = "kyma catalog Postgres: 5432 from inside the VPC only"
  vpc_id      = var.vpc_id

  ingress {
    description = "Postgres from the VPC (engine tasks/pods)"
    from_port   = 5432
    to_port     = 5432
    protocol    = "tcp"
    cidr_blocks = [var.vpc_cidr]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Project = var.name, ManagedBy = "terraform" }
}

resource "aws_db_instance" "this" {
  identifier     = "${var.name}-catalog"
  engine         = "postgres"
  engine_version = var.engine_version
  instance_class = var.instance_class

  allocated_storage = var.allocated_storage
  storage_type      = "gp3"
  storage_encrypted = true

  db_name  = var.db_name
  username = var.username
  password = var.password
  port     = 5432

  db_subnet_group_name   = aws_db_subnet_group.this.name
  vpc_security_group_ids = [aws_security_group.this.id]
  publicly_accessible    = false

  multi_az                = false
  backup_retention_period = 7
  deletion_protection     = false
  skip_final_snapshot     = true
  apply_immediately       = true

  tags = { Project = var.name, ManagedBy = "terraform" }
}
