---
title: ECS Fargate with Auto-Scaling
description: Deploy oidc-exchange on ECS Fargate with ALB, DynamoDB for users, and ElastiCache Valkey for sessions.
version: "0.2"
last_updated: 2026-03-26
---

# ECS Fargate with Auto-Scaling

Run oidc-exchange as an auto-scaling containerized service on AWS ECS Fargate, with an Application Load Balancer for traffic distribution, DynamoDB for user storage, and ElastiCache Valkey for low-latency session lookups.

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

| Resource | Purpose | Key settings |
|----------|---------|-------------|
| VPC | Network isolation | 2+ AZs, private subnets for Fargate, public subnets for ALB |
| ALB | TLS termination, health checks, traffic distribution | HTTPS listener, target group on port 8080 |
| ECS Cluster | Fargate task orchestration | Capacity provider: FARGATE |
| ECS Service | Desired count, auto-scaling | Min 2, max 20 tasks |
| DynamoDB | User storage | On-demand billing, single-table design |
| ElastiCache Valkey | Session storage | Serverless or single-node, in-VPC |
| KMS | JWT signing | ECC_NIST_P256 key |
| ECR | Container registry | Stores the oidc-exchange image |
| Secrets Manager | OIDC client secrets | Referenced by ECS task definition |

## Step-by-step

### 1. Build and push the container

```bash
# Build the image
docker build -t oidc-exchange .

# Create the ECR repository
aws ecr create-repository --repository-name oidc-exchange

# Login, tag, and push
ECR_URI=$(aws sts get-caller-identity --query Account --output text).dkr.ecr.$(aws configure get region).amazonaws.com
aws ecr get-login-password | docker login --username AWS --password-stdin $ECR_URI
docker tag oidc-exchange:latest $ECR_URI/oidc-exchange:latest
docker push $ECR_URI/oidc-exchange:latest
```

### 2. Create the DynamoDB table

```bash
aws dynamodb create-table \
  --table-name oidc-exchange \
  --attribute-definitions \
    AttributeName=PK,AttributeType=S \
    AttributeName=SK,AttributeType=S \
    AttributeName=GSI1pk,AttributeType=S \
    AttributeName=GSI1sk,AttributeType=S \
  --key-schema \
    AttributeName=PK,KeyType=HASH \
    AttributeName=SK,KeyType=RANGE \
  --global-secondary-indexes \
    'IndexName=GSI1,KeySchema=[{AttributeName=GSI1pk,KeyType=HASH},{AttributeName=GSI1sk,KeyType=RANGE}],Projection={ProjectionType=ALL}' \
  --billing-mode PAY_PER_REQUEST \
  --time-to-live-specification 'Enabled=true,AttributeName=ttl'
```

### 3. Create the ElastiCache Valkey cluster

For serverless (simplest, auto-scales):

```bash
aws elasticache create-serverless-cache \
  --serverless-cache-name oidc-exchange-sessions \
  --engine valkey \
  --subnet-group-name your-private-subnet-group \
  --security-group-ids sg-xxxxxxxxx
```

For a single-node cluster (predictable cost):

```bash
aws elasticache create-cache-cluster \
  --cache-cluster-id oidc-exchange-sessions \
  --engine redis \
  --cache-node-type cache.t4g.micro \
  --num-cache-nodes 1 \
  --cache-subnet-group-name your-private-subnet-group \
  --security-group-ids sg-xxxxxxxxx
```

Note the cluster endpoint for the configuration.

### 4. Create the KMS signing key

```bash
aws kms create-key \
  --key-spec ECC_NIST_P256 \
  --key-usage SIGN_VERIFY \
  --description "oidc-exchange JWT signing key"
```

### 5. Store secrets

```bash
aws secretsmanager create-secret \
  --name oidc-exchange/google-client-id \
  --secret-string "your-google-client-id"

aws secretsmanager create-secret \
  --name oidc-exchange/google-client-secret \
  --secret-string "your-google-client-secret"
```

### 6. Configure

Create `config/fargate.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "${KMS_KEY_ARN}"
algorithm = "ECDSA_SHA256"
kid = "prod-1"

# Users in DynamoDB
[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"

# Sessions in Valkey
[session_repository]
adapter = "valkey"

[session_repository.valkey]
url = "${VALKEY_URL}"
key_prefix = "oidc:"

[audit]
adapter = "sqs"

[audit.sqs]
queue_url = "${AUDIT_QUEUE_URL}"

[telemetry]
enabled = true
exporter = "otlp"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

### 7. Create the ECS task definition

```json
{
  "family": "oidc-exchange",
  "networkMode": "awsvpc",
  "requiresCompatibilities": ["FARGATE"],
  "cpu": "256",
  "memory": "512",
  "executionRoleArn": "arn:aws:iam::ACCOUNT:role/oidc-exchange-execution",
  "taskRoleArn": "arn:aws:iam::ACCOUNT:role/oidc-exchange-task",
  "containerDefinitions": [
    {
      "name": "oidc-exchange",
      "image": "ACCOUNT.dkr.ecr.REGION.amazonaws.com/oidc-exchange:latest",
      "portMappings": [
        {
          "containerPort": 8080,
          "protocol": "tcp"
        }
      ],
      "environment": [
        { "name": "OIDC_EXCHANGE_ENV", "value": "fargate" },
        { "name": "KMS_KEY_ARN", "value": "arn:aws:kms:..." },
        { "name": "VALKEY_URL", "value": "rediss://oidc-exchange-sessions.xxxxx.valkey.REGION.cache.amazonaws.com:6379" },
        { "name": "AUDIT_QUEUE_URL", "value": "https://sqs.REGION.amazonaws.com/ACCOUNT/oidc-exchange-audit" }
      ],
      "secrets": [
        {
          "name": "GOOGLE_CLIENT_ID",
          "valueFrom": "arn:aws:secretsmanager:REGION:ACCOUNT:secret:oidc-exchange/google-client-id"
        },
        {
          "name": "GOOGLE_CLIENT_SECRET",
          "valueFrom": "arn:aws:secretsmanager:REGION:ACCOUNT:secret:oidc-exchange/google-client-secret"
        }
      ],
      "healthCheck": {
        "command": ["CMD-SHELL", "curl -f http://localhost:8080/health || exit 1"],
        "interval": 30,
        "timeout": 5,
        "retries": 3,
        "startPeriod": 10
      },
      "logConfiguration": {
        "logDriver": "awslogs",
        "options": {
          "awslogs-group": "/ecs/oidc-exchange",
          "awslogs-region": "REGION",
          "awslogs-stream-prefix": "oidc-exchange"
        }
      }
    }
  ]
}
```

### 8. Create the ALB and ECS service

```bash
# Create the target group
aws elbv2 create-target-group \
  --name oidc-exchange \
  --protocol HTTP \
  --port 8080 \
  --vpc-id vpc-xxxxxxxxx \
  --target-type ip \
  --health-check-path /health \
  --health-check-interval-seconds 30

# Create the ALB
aws elbv2 create-load-balancer \
  --name oidc-exchange-alb \
  --subnets subnet-public-1 subnet-public-2 \
  --security-groups sg-alb \
  --scheme internet-facing

# Create HTTPS listener (requires ACM certificate)
aws elbv2 create-listener \
  --load-balancer-arn $ALB_ARN \
  --protocol HTTPS \
  --port 443 \
  --certificates CertificateArn=$ACM_CERT_ARN \
  --default-actions Type=forward,TargetGroupArn=$TG_ARN

# Create the ECS service
aws ecs create-service \
  --cluster default \
  --service-name oidc-exchange \
  --task-definition oidc-exchange \
  --desired-count 2 \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[subnet-private-1,subnet-private-2],securityGroups=[sg-task],assignPublicIp=DISABLED}" \
  --load-balancers "targetGroupArn=$TG_ARN,containerName=oidc-exchange,containerPort=8080" \
  --deployment-configuration "minimumHealthyPercent=100,maximumPercent=200"
```

### 9. Configure auto-scaling

```bash
# Register the scalable target
aws application-autoscaling register-scalable-target \
  --service-namespace ecs \
  --resource-id service/default/oidc-exchange \
  --scalable-dimension ecs:service:DesiredCount \
  --min-capacity 2 \
  --max-capacity 20

# Scale on CPU utilization
aws application-autoscaling put-scaling-policy \
  --service-namespace ecs \
  --resource-id service/default/oidc-exchange \
  --scalable-dimension ecs:service:DesiredCount \
  --policy-name cpu-tracking \
  --policy-type TargetTrackingScaling \
  --target-tracking-scaling-policy-configuration '{
    "TargetValue": 60.0,
    "PredefinedMetricSpecification": {
      "PredefinedMetricType": "ECSServiceAverageCPUUtilization"
    },
    "ScaleInCooldown": 300,
    "ScaleOutCooldown": 60
  }'
```

This scales up when average CPU across tasks exceeds 60%, and scales in after 5 minutes of reduced load.

## IAM policies

### Task execution role

Needs permissions to pull the image, read secrets, and write logs:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "ecr:GetAuthorizationToken",
        "ecr:BatchGetImage",
        "ecr:GetDownloadUrlForLayer"
      ],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": ["secretsmanager:GetSecretValue"],
      "Resource": "arn:aws:secretsmanager:*:*:secret:oidc-exchange/*"
    },
    {
      "Effect": "Allow",
      "Action": ["logs:CreateLogStream", "logs:PutLogEvents"],
      "Resource": "arn:aws:logs:*:*:log-group:/ecs/oidc-exchange:*"
    }
  ]
}
```

### Task role

Needs permissions for the application to access DynamoDB, KMS, Valkey (via VPC), and SQS:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "dynamodb:GetItem",
        "dynamodb:PutItem",
        "dynamodb:UpdateItem",
        "dynamodb:DeleteItem",
        "dynamodb:Query",
        "dynamodb:BatchWriteItem"
      ],
      "Resource": [
        "arn:aws:dynamodb:*:*:table/oidc-exchange",
        "arn:aws:dynamodb:*:*:table/oidc-exchange/index/GSI1"
      ]
    },
    {
      "Effect": "Allow",
      "Action": ["kms:Sign", "kms:GetPublicKey"],
      "Resource": "arn:aws:kms:*:*:key/KEY_ID"
    },
    {
      "Effect": "Allow",
      "Action": ["sqs:SendMessage"],
      "Resource": "arn:aws:sqs:*:*:oidc-exchange-audit"
    }
  ]
}
```

ElastiCache Valkey access is controlled via security groups (VPC networking), not IAM.

## Security groups

| Resource | Inbound | Outbound |
|----------|---------|----------|
| ALB (`sg-alb`) | 443/tcp from 0.0.0.0/0 | 8080/tcp to `sg-task` |
| Fargate tasks (`sg-task`) | 8080/tcp from `sg-alb` | 443/tcp to 0.0.0.0/0 (OIDC providers, AWS APIs) |
| Fargate tasks (`sg-task`) | — | 6379/tcp to `sg-valkey` |
| ElastiCache (`sg-valkey`) | 6379/tcp from `sg-task` | — |

DynamoDB, KMS, SQS, and Secrets Manager are accessed via AWS service endpoints (HTTPS over port 443). For private networking, add VPC endpoints for these services.

## Cost optimization

- **Fargate Spot**: for non-critical environments, use `capacityProviderStrategy` with `FARGATE_SPOT` for up to 70% savings. The ALB health checks and ECS service will replace interrupted tasks automatically.
- **ElastiCache Serverless**: auto-scales with usage. For predictable workloads, a reserved `cache.t4g.micro` node is cheaper.
- **DynamoDB on-demand**: no capacity planning needed. For steady-state traffic, consider switching to provisioned capacity with auto-scaling.
- **ARM64**: build the container for `linux/arm64` and set the task definition to use ARM64 for ~20% lower Fargate cost.
