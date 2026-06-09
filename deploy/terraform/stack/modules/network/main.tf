# Minimal 2-AZ network for a single public-facing service: VPC with public
# subnets (no NAT gateway — saves ~$32/mo; tasks/nodes get a public IP for
# egress to S3/Supabase/GHCR/EKS-API) + private subnets (RDS only), an
# optional ALB, and optional ACM + Route53 wiring.
#
# `create_alb` gates the ALB and its security groups: the Fargate target uses
# the ALB; the EKS target sets it false and routes traffic through a Kubernetes
# ingress instead.

data "aws_availability_zones" "available" {
  state = "available"
}

resource "aws_vpc" "this" {
  cidr_block           = "10.20.0.0/16"
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = { Name = "${var.name}-vpc", ManagedBy = "terraform" }
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = "${var.name}-igw" }
}

resource "aws_subnet" "public" {
  count = 2

  vpc_id                  = aws_vpc.this.id
  cidr_block              = cidrsubnet(aws_vpc.this.cidr_block, 8, count.index)
  availability_zone       = data.aws_availability_zones.available.names[count.index]
  map_public_ip_on_launch = true
  tags = {
    Name = "${var.name}-public-${count.index}"
    # EKS uses this tag to place public load balancers.
    "kubernetes.io/role/elb" = "1"
  }
}

# Private subnets host RDS (not publicly accessible). No NAT — they have no
# default route to the internet, which is fine: only intra-VPC reaches them.
resource "aws_subnet" "private" {
  count = 2

  vpc_id            = aws_vpc.this.id
  cidr_block        = cidrsubnet(aws_vpc.this.cidr_block, 8, count.index + 10)
  availability_zone = data.aws_availability_zones.available.names[count.index]
  tags              = { Name = "${var.name}-private-${count.index}" }
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.this.id
  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.this.id
  }
  tags = { Name = "${var.name}-public" }
}

resource "aws_route_table_association" "public" {
  count = 2

  subnet_id      = aws_subnet.public[count.index].id
  route_table_id = aws_route_table.public.id
}

resource "aws_route_table" "private" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = "${var.name}-private" }
}

resource "aws_route_table_association" "private" {
  count = 2

  subnet_id      = aws_subnet.private[count.index].id
  route_table_id = aws_route_table.private.id
}

# ---------------------------------------------------------------------------
# Security groups (ALB path only)
# ---------------------------------------------------------------------------

resource "aws_security_group" "alb" {
  count       = var.create_alb ? 1 : 0
  name        = "${var.name}-alb"
  description = "Public HTTP(S) into the kyma ALB"
  vpc_id      = aws_vpc.this.id

  ingress {
    description = "HTTP"
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  ingress {
    description = "HTTPS"
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  egress {
    description = "To the engine tasks"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_security_group" "service" {
  count       = var.create_alb ? 1 : 0
  name        = "${var.name}-engine"
  description = "kyma engine tasks: ALB in, internet out (S3/Supabase/GHCR)"
  vpc_id      = aws_vpc.this.id

  ingress {
    description     = "Engine HTTP from the ALB only"
    from_port       = 8080
    to_port         = 8080
    protocol        = "tcp"
    security_groups = [aws_security_group.alb[0].id]
  }

  egress {
    description = "All egress (image pull, S3, Supabase)"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

# ---------------------------------------------------------------------------
# ALB + target group (Fargate only)
# ---------------------------------------------------------------------------

resource "aws_lb" "this" {
  count              = var.create_alb ? 1 : 0
  name               = "${var.name}-alb"
  load_balancer_type = "application"
  security_groups    = [aws_security_group.alb[0].id]
  subnets            = aws_subnet.public[*].id
  tags               = { ManagedBy = "terraform" }
}

resource "aws_lb_target_group" "engine" {
  count       = var.create_alb ? 1 : 0
  name        = "${var.name}-engine"
  port        = 8080
  protocol    = "HTTP"
  target_type = "ip"
  vpc_id      = aws_vpc.this.id

  health_check {
    path                = "/health"
    matcher             = "200"
    interval            = 15
    healthy_threshold   = 2
    unhealthy_threshold = 5
  }

  deregistration_delay = 30
}

# ---------------------------------------------------------------------------
# TLS: with a domain we issue an ACM cert; with a Route53 zone its validation
# and the A-record are fully automated. Without a zone, `apply` waits while
# the operator creates the printed validation CNAME at their DNS provider.
# ---------------------------------------------------------------------------

locals {
  https = var.create_alb && var.domain != ""
}

resource "aws_acm_certificate" "this" {
  count = local.https ? 1 : 0

  domain_name       = var.domain
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_route53_record" "validation" {
  for_each = local.https && var.route53_zone_id != "" ? {
    for dvo in aws_acm_certificate.this[0].domain_validation_options :
    dvo.domain_name => {
      name   = dvo.resource_record_name
      type   = dvo.resource_record_type
      record = dvo.resource_record_value
    }
  } : {}

  zone_id = var.route53_zone_id
  name    = each.value.name
  type    = each.value.type
  records = [each.value.record]
  ttl     = 300
}

resource "aws_acm_certificate_validation" "this" {
  count = local.https ? 1 : 0

  certificate_arn = aws_acm_certificate.this[0].arn
  validation_record_fqdns = var.route53_zone_id != "" ? [
    for r in aws_route53_record.validation : r.fqdn
  ] : null
}

resource "aws_route53_record" "engine" {
  count = local.https && var.route53_zone_id != "" ? 1 : 0

  zone_id = var.route53_zone_id
  name    = var.domain
  type    = "A"

  alias {
    name                   = aws_lb.this[0].dns_name
    zone_id                = aws_lb.this[0].zone_id
    evaluate_target_health = true
  }
}

# HTTPS listener (with domain) — HTTP redirects to it.
resource "aws_lb_listener" "https" {
  count = local.https ? 1 : 0

  load_balancer_arn = aws_lb.this[0].arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = "ELBSecurityPolicy-TLS13-1-2-2021-06"
  certificate_arn   = aws_acm_certificate_validation.this[0].certificate_arn

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.engine[0].arn
  }
}

resource "aws_lb_listener" "http_redirect" {
  count = local.https ? 1 : 0

  load_balancer_arn = aws_lb.this[0].arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type = "redirect"
    redirect {
      port        = "443"
      protocol    = "HTTPS"
      status_code = "HTTP_301"
    }
  }
}

# Plain-HTTP listener (no domain) — demo-grade, documented as such.
resource "aws_lb_listener" "http" {
  count = var.create_alb && var.domain == "" ? 1 : 0

  load_balancer_arn = aws_lb.this[0].arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.engine[0].arn
  }
}
