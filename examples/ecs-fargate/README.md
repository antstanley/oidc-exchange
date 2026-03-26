---
title: "ECS Fargate Example"
description: "Deploy oidc-exchange on AWS ECS Fargate with DynamoDB, ElastiCache Valkey, KMS, and SQS"
version: "0.1"
last_updated: 2026-03-26
---

# ECS Fargate Example

## Architecture

```
ALB (internet-facing)
  -> ECS Fargate (auto-scaling 2-10 tasks)
       -> DynamoDB (users / repository)
       -> ElastiCache Valkey (sessions)
       -> KMS (JWT signing, ECC_NIST_P256)
       -> SQS (audit events)
```

An Application Load Balancer receives traffic and forwards it to ECS Fargate
tasks running `oidc-exchange`. The service auto-scales based on CPU utilization.
DynamoDB stores user and token data, ElastiCache Valkey handles sessions, KMS
provides asymmetric signing keys for JWTs, and SQS captures audit events.

## Prerequisites

- Terraform 1.5+
- AWS CLI (configured with appropriate credentials)
- Docker
- cargo (to build oidc-exchange)
- A domain with an ACM certificate (or use the HTTP listener for testing)

## Quick Start

1. **Build the Docker image** (from the repository root):

   ```bash
   docker build -t oidc-exchange -f examples/ecs-fargate/Dockerfile .
   ```

2. **Create an ECR repository and push the image**:

   ```bash
   aws ecr get-login-password --region us-east-1 | docker login --username AWS --password-stdin 123456789012.dkr.ecr.us-east-1.amazonaws.com
   aws ecr create-repository --repository-name oidc-exchange --region us-east-1
   docker tag oidc-exchange:latest 123456789012.dkr.ecr.us-east-1.amazonaws.com/oidc-exchange:latest
   docker push 123456789012.dkr.ecr.us-east-1.amazonaws.com/oidc-exchange:latest
   ```

   Replace `123456789012` with your AWS account ID.

3. **Configure Terraform variables**:

   ```bash
   cd examples/ecs-fargate/infra
   cp terraform.tfvars.example terraform.tfvars
   ```

   Edit `terraform.tfvars` and fill in your values.

4. **Deploy the infrastructure**:

   ```bash
   terraform init && terraform plan && terraform apply
   ```

5. **Test the deployment**:

   ```bash
   ALB_URL=$(terraform output -raw alb_url)
   curl -s "$ALB_URL/health"
   curl -s "$ALB_URL/.well-known/openid-configuration" | jq .
   ```

## Cleanup

```bash
terraform destroy
```

This will remove all AWS resources created by this example.

## Further Reading

See [docs/integration/ecs-fargate.md](../../docs/integration/ecs-fargate.md) for a detailed explanation of this deployment pattern.
