output "vpc_id" {
  value = aws_vpc.this.id
}

output "public_subnet_ids" {
  value = aws_subnet.public[*].id
}

output "alb_dns_name" {
  value = aws_lb.this.dns_name
}

output "target_group_arn" {
  value = aws_lb_target_group.engine.arn
}

output "service_security_group_id" {
  value = aws_security_group.service.id
}

output "acm_validation_records" {
  value = var.domain != "" && var.route53_zone_id == "" ? [
    for dvo in aws_acm_certificate.this[0].domain_validation_options : {
      name  = dvo.resource_record_name
      type  = dvo.resource_record_type
      value = dvo.resource_record_value
    }
  ] : []
  description = "DNS records to create manually for ACM validation (manual-DNS path only)."
}
