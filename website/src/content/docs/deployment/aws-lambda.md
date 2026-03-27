---
title: AWS Lambda
description: Deploy oidc-exchange as a serverless Lambda function with DynamoDB and KMS.
---

This is the recommended deployment for AWS-native workloads. oidc-exchange detects the `AWS_LAMBDA_RUNTIME_API` environment variable at startup and automatically switches to Lambda mode — no code or config changes needed.

A runnable example is in [`examples/aws-web/`](https://github.com/example/oidc-exchange/tree/main/examples/aws-web).

## Infrastructure

A complete CDK example is in `examples/aws-web/infra/`. The key resources are:

| Resource | Purpose |
|----------|---------|
| DynamoDB table | User and session storage (single-table design, on-demand billing) |
| KMS key (ECC_NIST_P256) | JWT signing — keys never leave KMS |
| CloudTrail Lake event data store | Audit trail (optional) |
| Lambda function (`provided.al2023`) | The oidc-exchange binary |
| API Gateway HTTP API | Routes traffic to Lambda |

## Step-by-step

**1. Build the Lambda bootstrap binary**

```bash
cargo lambda build --release
```

This produces `target/lambda/oidc-exchange/bootstrap`, a statically linked binary compatible with the `provided.al2023` Lambda runtime.

**2. Create the DynamoDB table**

The table uses a single-table design with a partition key (`PK`), sort key (`SK`), and one global secondary index (`GSI1`). See `schemas/dynamodb/table-design.json` for the full schema.

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

**3. Create the KMS signing key**

```bash
aws kms create-key \
  --key-spec ECC_NIST_P256 \
  --key-usage SIGN_VERIFY \
  --description "oidc-exchange JWT signing key"
```

Note the key ARN for the configuration.

**4. Configure**

Create `config/lambda.toml`:

```toml
[server]
issuer = "https://auth.example.com"

[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "${KMS_KEY_ARN}"
algorithm = "ECDSA_SHA256"
kid = "prod-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"

[audit]
adapter = "cloudtrail"
blocking_threshold = "error"

[audit.cloudtrail]
channel_arn = "${CLOUDTRAIL_CHANNEL_ARN}"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

**5. Deploy the Lambda function**

```bash
# Copy the bootstrap binary
cp target/lambda/oidc-exchange/bootstrap deploy/

# Create the function
aws lambda create-function \
  --function-name oidc-exchange \
  --runtime provided.al2023 \
  --handler bootstrap \
  --architectures arm64 \
  --zip-file fileb://deploy/bootstrap.zip \
  --role arn:aws:iam::123456789012:role/oidc-exchange-role \
  --memory-size 256 \
  --timeout 29 \
  --environment "Variables={OIDC_EXCHANGE_ENV=lambda,GOOGLE_CLIENT_ID=...,GOOGLE_CLIENT_SECRET=...,KMS_KEY_ARN=...,CLOUDTRAIL_CHANNEL_ARN=...}"
```

Or use the CDK stack in `examples/aws-web/infra/` for a fully automated deployment:

```bash
cd examples/aws-web/infra
npm install
npx cdk deploy \
  -c googleClientId="your-client-id" \
  -c googleClientSecret="your-client-secret"
```

**6. Create the API Gateway route**

Route `/auth/{proxy+}` to the Lambda function. The CDK example does this automatically. For manual setup:

```bash
aws apigatewayv2 create-api \
  --name oidc-exchange \
  --protocol-type HTTP

# Add Lambda integration and routes for /auth/{proxy+}
```

## IAM permissions

The Lambda execution role needs:

```json
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
}
```

Plus `kms:Sign`, `kms:GetPublicKey` on the signing key, and optionally `cloudtrail-data:PutAuditEvents` on the CloudTrail channel.

## Cold start considerations

The `provided.al2023` runtime with a Rust binary typically cold-starts in 50-150ms. KMS signing adds ~20ms per request. To minimize cold starts:

- Use ARM64 (`arm64` architecture) for lower cost and comparable performance
- Set memory to 256 MB or higher — Lambda allocates CPU proportionally to memory
- Use provisioned concurrency if sub-100ms p99 latency is required
