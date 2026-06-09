output "vpc_id" {
  value = aws_vpc.this.id
}

output "vpc_cidr" {
  value = aws_vpc.this.cidr_block
}

output "public_subnet_ids" {
  value = aws_subnet.public[*].id
}

output "private_subnet_ids" {
  value = aws_subnet.private[*].id
}

output "alb_dns_name" {
  value = var.create_alb ? aws_lb.this[0].dns_name : ""
}

output "target_group_arn" {
  value = var.create_alb ? aws_lb_target_group.engine[0].arn : ""
}

output "service_security_group_id" {
  value = var.create_alb ? aws_security_group.service[0].id : ""
}

output "acm_validation_records" {
  value = local.https && var.route53_zone_id == "" ? [
    for dvo in aws_acm_certificate.this[0].domain_validation_options : {
      name  = dvo.resource_record_name
      type  = dvo.resource_record_type
      value = dvo.resource_record_value
    }
  ] : []
  description = "DNS records to create manually for ACM validation (manual-DNS path only)."
}
