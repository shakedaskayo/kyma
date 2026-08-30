# The pensieve engine on ECS Fargate: one cluster, one task definition, one
# service behind the ALB target group. ARM64 (Graviton) by default — the
# release images are multi-arch and ARM is ~20% cheaper.

resource "aws_ecs_cluster" "this" {
  name = "${var.name}-cluster"

  setting {
    name  = "containerInsights"
    value = "disabled" # paid; enable when you need task-level metrics
  }
}

resource "aws_cloudwatch_log_group" "engine" {
  name              = "/ecs/${var.name}-engine"
  retention_in_days = 30
}

# ---------------------------------------------------------------------------
# IAM: execution role (pull image, write logs, read SSM secrets) and task
# role (the engine's own AWS access — scoped to the extent bucket).
# ---------------------------------------------------------------------------

data "aws_iam_policy_document" "assume_ecs" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "execution" {
  name               = "${var.name}-engine-execution"
  assume_role_policy = data.aws_iam_policy_document.assume_ecs.json
}

resource "aws_iam_role_policy_attachment" "execution_managed" {
  role       = aws_iam_role.execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

data "aws_kms_alias" "ssm" {
  name = "alias/aws/ssm"
}

data "aws_iam_policy_document" "execution_secrets" {
  statement {
    sid       = "ReadPensieveParams"
    actions   = ["ssm:GetParameters"]
    resources = values(var.ssm_param_arns)
  }
  statement {
    sid       = "DecryptSsmDefaultKey"
    actions   = ["kms:Decrypt"]
    resources = [data.aws_kms_alias.ssm.target_key_arn]
  }
}

resource "aws_iam_role_policy" "execution_secrets" {
  name   = "read-pensieve-secrets"
  role   = aws_iam_role.execution.id
  policy = data.aws_iam_policy_document.execution_secrets.json
}

resource "aws_iam_role" "task" {
  name               = "${var.name}-engine-task"
  assume_role_policy = data.aws_iam_policy_document.assume_ecs.json
}

# Keyless S3: the engine resolves these credentials via the standard AWS
# provider chain (no PENSIEVE_S3_ACCESS_KEY_ID needed). Only for the native-S3
# storage backend — Supabase Storage authenticates with its own keys.
data "aws_iam_policy_document" "task_s3" {
  count = var.s3_bucket_arn != "" ? 1 : 0

  statement {
    sid       = "ExtentObjects"
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload"]
    resources = ["${var.s3_bucket_arn}/*"]
  }
  statement {
    sid       = "ExtentList"
    actions   = ["s3:ListBucket", "s3:GetBucketLocation"]
    resources = [var.s3_bucket_arn]
  }
}

resource "aws_iam_role_policy" "task_s3" {
  count = var.s3_bucket_arn != "" ? 1 : 0

  name   = "extent-bucket-access"
  role   = aws_iam_role.task.id
  policy = data.aws_iam_policy_document.task_s3[0].json
}

# ---------------------------------------------------------------------------
# Task definition + service
# ---------------------------------------------------------------------------

resource "aws_ecs_task_definition" "engine" {
  family                   = "${var.name}-engine"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.task_cpu
  memory                   = var.task_memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.task.arn

  runtime_platform {
    operating_system_family = "LINUX"
    cpu_architecture        = "ARM64"
  }

  container_definitions = jsonencode([
    {
      name      = "pensieve-engine"
      image     = var.image
      essential = true

      portMappings = [
        { containerPort = 8080, protocol = "tcp" }
      ]

      environment = [
        for k, v in var.environment : { name = k, value = v }
      ]

      secrets = [
        for k, arn in var.secrets : { name = k, valueFrom = arn }
      ]

      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.engine.name
          "awslogs-region"        = var.aws_region
          "awslogs-stream-prefix" = "engine"
        }
      }
    }
  ])
}

resource "aws_ecs_service" "engine" {
  name            = "${var.name}-engine"
  cluster         = aws_ecs_cluster.this.id
  task_definition = aws_ecs_task_definition.engine.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  # Migrations + catalog connect can take a moment on first boot.
  health_check_grace_period_seconds = 90

  network_configuration {
    subnets          = var.subnet_ids
    security_groups  = [var.service_sg_id]
    assign_public_ip = true # public subnet, no NAT — egress for S3/Supabase/GHCR
  }

  load_balancer {
    target_group_arn = var.target_group
    container_name   = "pensieve-engine"
    container_port   = 8080
  }
}
