variable "project_name" {
  description = "Project name used for resource naming"
  type        = string
  default     = "oidc-exchange"
}

variable "aws_region" {
  description = "AWS region"
  type        = string
  default     = "us-east-1"
}

variable "container_image" {
  description = "ECR image URI for oidc-exchange"
  type        = string
}

variable "google_client_id" {
  description = "Google OAuth client ID"
  type        = string
  sensitive   = true
}

variable "google_client_secret" {
  description = "Google OAuth client secret"
  type        = string
  sensitive   = true
}

variable "issuer_url" {
  description = "JWT issuer URL (e.g., https://auth.example.com or the ALB URL)"
  type        = string
}

variable "certificate_arn" {
  description = "ACM certificate ARN for HTTPS. Leave empty for HTTP-only (testing)."
  type        = string
  default     = ""
}

variable "desired_count" {
  description = "Desired number of Fargate tasks"
  type        = number
  default     = 2
}

variable "max_count" {
  description = "Maximum number of Fargate tasks for auto-scaling"
  type        = number
  default     = 10
}
