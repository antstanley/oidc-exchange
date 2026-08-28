---
title: AWS Lambda
description: Deploy oidc-exchange as a serverless Lambda function with DynamoDB and KMS.
---

This is the recommended deployment for AWS-native workloads. oidc-exchange detects the `AWS_LAMBDA_RUNTIME_API` environment variable at startup and automatically switches to Lambda mode, with no code or config changes needed.

A runnable example is in [`examples/aws-web/`](https://github.com/example/oidc-exchange/tree/main/examples/aws-web).

## Infrastructure

A complete CDK example is in `examples/aws-web/infra/`. The key resources are:

| Resource | Purpose |
|----------|---------|
| DynamoDB table | User and session storage (single-table design, on-demand billing) |
| KMS key (ECC_NIST_P256) | JWT signing (keys never leave KMS) |
| SQS queue | Audit trail (optional) |
| Lambda function (`provided.al2023`) | The oidc-exchange binary |
| API Gateway HTTP API | Routes traffic to Lambda |

## Step-by-step

**1. Build the Lambda bootstrap binary**

```bash
cargo lambda build --release
```

This produces `target/lambda/oidc-exchange/bootstrap`, a statically linked binary compatible with the `provided.al2023` Lambda runtime.

**2. Create the DynamoDB table**

The table uses a single-table design with a partition key (`pk`), sort key (`sk`), and one global secondary index (`GSI1`). The base-table key attributes are lowercase `pk`/`sk`; the GSI attributes are `GSI1pk`/`GSI1sk`. These names are case-sensitive and must match exactly. See `schemas/dynamodb/table-design.json` for the full schema.

```bash
aws dynamodb create-table \
  --table-name oidc-exchange \
  --attribute-definitions \
    AttributeName=pk,AttributeType=S \
    AttributeName=sk,AttributeType=S \
    AttributeName=GSI1pk,AttributeType=S \
    AttributeName=GSI1sk,AttributeType=S \
  --key-schema \
    AttributeName=pk,KeyType=HASH \
    AttributeName=sk,KeyType=RANGE \
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
algorithm = "ES256"
kid = "prod-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"

[audit]
adapter = "sqs"
durability = "enforce"
emit_threshold = "info"

[audit.sqs]
queue_url = "${AUDIT_QUEUE_URL}"

[rate_limit]
enabled = true
store = "in_process"
window = "1m"
per_ip = 60
per_ip_failures = 10
per_subject = 10
per_provider = 600

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
# Origins Google's discovery document may name beyond the issuer's origin:
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
```

`endpoint_origins` pins which origins a provider's discovery document is allowed to name; each entry must be a bare `https://host[:port]`, and an unpinned origin logs a warning when discovered (see [Identity Providers](/guides/providers/)).

Note that `rate_limit.store = "in_process"` holds budgets per Lambda execution environment, not shared across them. Under concurrency the effective limit is roughly N times the configured value, where N is the number of warm execution environments serving traffic.

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
  --environment "Variables={OIDC_EXCHANGE_ENV=lambda,GOOGLE_CLIENT_ID=...,KMS_KEY_ARN=...,AUDIT_QUEUE_URL=...}"
```

Do not pass `GOOGLE_CLIENT_SECRET` (or any secret) as a plaintext `Variables=` entry: it would be stored in the function configuration and returned by every `GetFunctionConfiguration` call. Deliver it by reference instead. Store the value in Secrets Manager, then fetch it at cold start with a short `AWS_LAMBDA_EXEC_WRAPPER` script (or the AWS Parameters and Secrets Lambda Extension) that exports `GOOGLE_CLIENT_SECRET` into the process environment before `bootstrap` runs. The service then resolves the `${GOOGLE_CLIENT_SECRET}` placeholder in `config/lambda.toml` from that process environment.

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

Plus `kms:Sign`, `kms:GetPublicKey` on the signing key, and optionally `sqs:SendMessage` on the audit queue.

## Cold start considerations

The `provided.al2023` runtime with a Rust binary typically cold-starts in 50-150ms. KMS signing adds ~20ms per request. To minimize cold starts:

- Use ARM64 (`arm64` architecture) for lower cost and comparable performance
- Set memory to 256 MB or higher; Lambda allocates CPU proportionally to memory
- Use provisioned concurrency if sub-100ms p99 latency is required
