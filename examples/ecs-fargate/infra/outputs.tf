output "alb_url" {
  description = "ALB DNS name"
  value       = "http://${aws_lb.main.dns_name}"
}

output "dynamodb_table" {
  value = aws_dynamodb_table.main.name
}

output "ecr_repository_url" {
  value = aws_ecr_repository.main.repository_url
}

output "kms_key_arn" {
  value = aws_kms_key.signing.arn
}

output "valkey_endpoint" {
  value = aws_elasticache_replication_group.valkey.primary_endpoint_address
}

output "sqs_queue_url" {
  value = aws_sqs_queue.audit.url
}
