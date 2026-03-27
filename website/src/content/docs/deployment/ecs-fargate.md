---
title: ECS Fargate
description: Deploy oidc-exchange on ECS Fargate with ALB, DynamoDB, and ElastiCache Valkey using Terraform.
---

Run oidc-exchange as an auto-scaling containerized service on AWS ECS Fargate, with an Application Load Balancer for traffic distribution, DynamoDB for user storage, and ElastiCache Valkey for low-latency session lookups.

Infrastructure is managed with Terraform. A runnable example is in [`examples/ecs-fargate/`](https://github.com/example/oidc-exchange/tree/main/examples/ecs-fargate).

## When to use this

- You need high availability with automatic scaling
- You want managed infrastructure without managing EC2 instances or Kubernetes
- You want the performance of Valkey for session operations (sub-millisecond) with the durability of DynamoDB for user records
- You are already invested in the AWS ecosystem

## Architecture

```
                    ┌──────────────────┐
                    │       ALB        │
                    │ (TLS termination)│
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
        ┌─────┴─────┐ ┌─────┴─────┐ ┌─────┴─────┐
        │  Fargate   │ │  Fargate   │ │  Fargate   │
        │  Task 1    │ │  Task 2    │ │  Task N    │
        └──┬─────┬───┘ └──┬─────┬───┘ └──┬─────┬───┘
           │     │         │     │         │     │
    ┌──────┴──┐  │  ┌──────┴──┐  │  ┌──────┴──┐  │
    │DynamoDB │  │  │DynamoDB │  │  │DynamoDB │  │
    │ (users) │  │  │ (users) │  │  │ (users) │  │
    └─────────┘  │  └─────────┘  │  └─────────┘  │
           ┌─────┴─────┐  ┌─────┴─────┐  ┌─────┴─────┐
           │ElastiCache │  │ElastiCache │  │ElastiCache │
           │  Valkey    │  │  Valkey    │  │  Valkey    │
           │(sessions)  │  │(sessions)  │  │(sessions)  │
           └────────────┘  └────────────┘  └────────────┘
```

All Fargate tasks share the same DynamoDB table and ElastiCache cluster.

## Infrastructure

| Resource | Purpose | Managed by |
|----------|---------|-----------|
| VPC | Network isolation (2 AZs, public + private subnets) | Terraform |
| ALB | TLS termination, health checks, traffic distribution | Terraform |
| ECS Cluster + Service | Fargate task orchestration, auto-scaling (2-20 tasks) | Terraform |
| DynamoDB | User storage (on-demand billing, single-table design) | Terraform |
| ElastiCache Valkey | Session storage (single-node, in-VPC) | Terraform |
| KMS | JWT signing (ECC_NIST_P256) | Terraform |
| SQS | Audit event queue | Terraform |
| ECR | Container registry | Terraform |
| Secrets Manager | OIDC client secrets | Terraform |

## Prerequisites

- [Terraform](https://www.terraform.io/) 1.5+
- AWS CLI configured with credentials
- Docker (to build and push the container image)
- A Google OAuth client ID and secret (or another OIDC provider)
- (Optional) An ACM certificate ARN for HTTPS

## Deployment

### 1. Build and push the container image

From the repository root:

```bash
# Build the image
docker build -t oidc-exchange -f examples/ecs-fargate/Dockerfile .

# The Terraform config creates an ECR repository. If deploying for the first time,
# run terraform apply first (step 3), then push:
ECR_URL=$(terraform -chdir=examples/ecs-fargate/infra output -raw ecr_repository_url)
aws ecr get-login-password --region us-east-1 | docker login --username AWS --password-stdin $ECR_URL
docker tag oidc-exchange:latest $ECR_URL:latest
docker push $ECR_URL:latest
```

### 2. Configure Terraform variables

```bash
cd examples/ecs-fargate/infra
cp terraform.tfvars.example terraform.tfvars
```

Edit `terraform.tfvars`:

```hcl
aws_region           = "us-east-1"
container_image      = "123456789012.dkr.ecr.us-east-1.amazonaws.com/oidc-exchange:latest"
google_client_id     = "your-google-client-id"
google_client_secret = "your-google-client-secret"
issuer_url           = "https://auth.example.com"
# certificate_arn    = "arn:aws:acm:us-east-1:123456789012:certificate/xxx"
```

### 3. Deploy

```bash
terraform init
terraform plan
terraform apply
```

Terraform creates all infrastructure: VPC, subnets, NAT gateway, ALB, ECS cluster and service, DynamoDB table, ElastiCache Valkey cluster, KMS key, SQS queue, IAM roles, security groups, and auto-scaling policies.

### 4. Verify

```bash
ALB_URL=$(terraform output -raw alb_url)
curl $ALB_URL/health
curl $ALB_URL/.well-known/openid-configuration
```

## Configuration

The TOML config at `examples/ecs-fargate/config/fargate.toml` uses environment variable placeholders. Terraform injects the actual values via the ECS task definition:

```toml
[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "${DYNAMODB_TABLE_NAME}"

[session_repository]
adapter = "valkey"

[session_repository.valkey]
url = "${VALKEY_URL}"
key_prefix = "oidc:"
```

When `[session_repository]` is present, sessions use Valkey while users stay in DynamoDB. Remove the `[session_repository]` section to store everything in DynamoDB.

## Auto-scaling

The Terraform configuration sets up CPU-based target tracking:

- **Target**: 60% average CPU utilization
- **Min tasks**: 2 (configurable via `desired_count`)
- **Max tasks**: 20 (configurable via `max_count`)
- **Scale-out cooldown**: 60 seconds
- **Scale-in cooldown**: 300 seconds

Modify `variables.tf` defaults or override in `terraform.tfvars` to tune scaling behavior.

## Security groups

| Resource | Inbound | Outbound |
|----------|---------|----------|
| ALB (`sg-alb`) | 80,443/tcp from 0.0.0.0/0 | 8080/tcp to `sg-task` |
| Fargate tasks (`sg-task`) | 8080/tcp from `sg-alb` | 443/tcp to 0.0.0.0/0 (OIDC providers, AWS APIs) |
| Fargate tasks (`sg-task`) | — | 6379/tcp to `sg-valkey` |
| ElastiCache (`sg-valkey`) | 6379/tcp from `sg-task` | — |

DynamoDB, KMS, SQS, and Secrets Manager are accessed via AWS service endpoints (HTTPS over port 443). For private networking, add VPC endpoints for these services.

## IAM roles

Terraform creates two IAM roles:

**Task execution role** — used by ECS to pull images, read secrets, and write logs:
- `ecr:GetAuthorizationToken`, `ecr:BatchGetImage`, `ecr:GetDownloadUrlForLayer`
- `secretsmanager:GetSecretValue` (scoped to `oidc-exchange/*`)
- `logs:CreateLogStream`, `logs:PutLogEvents`

**Task role** — used by the running container to access AWS services:
- DynamoDB: `GetItem`, `PutItem`, `UpdateItem`, `DeleteItem`, `Query`, `BatchWriteItem`
- KMS: `Sign`, `GetPublicKey`
- SQS: `SendMessage`

ElastiCache Valkey access is controlled via security groups (VPC networking), not IAM.

## Cost optimization

- **Fargate Spot**: add `capacity_provider_strategy` with `FARGATE_SPOT` in the ECS service for up to 70% savings
- **ElastiCache**: the example uses a single `cache.t4g.micro` node; upgrade to a replication group for HA, or use ElastiCache Serverless for auto-scaling
- **DynamoDB on-demand**: no capacity planning needed; switch to provisioned capacity for steady-state traffic
- **ARM64**: build for `linux/arm64` and set `runtime_platform` in the task definition for ~20% lower Fargate cost

## Cleanup

```bash
terraform destroy
```

This removes all AWS resources created by the example.
