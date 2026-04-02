# AWS Lambda + oidc-exchange

Deploys oidc-exchange as an AWS Lambda function using `@oidc-exchange/lambda`.

The handler automatically detects the event source — API Gateway v1, API Gateway v2 (HTTP API), Lambda Function URL, or ALB — and translates the event into an HTTP request.

## Setup

```bash
mkdir -p keys
openssl genpkey -algorithm Ed25519 -out keys/signing.pem
pnpm install
pnpm build
```

## Deploy

Use your preferred deployment tool (SAM, CDK, Serverless Framework, Terraform) to deploy `handler.handler` as a Lambda function with Node.js 22 runtime.

### SAM template example

```yaml
Resources:
  OidcExchangeFunction:
    Type: AWS::Serverless::Function
    Properties:
      Handler: handler.handler
      Runtime: nodejs22.x
      CodeUri: .
      Events:
        Auth:
          Type: HttpApi
          Properties:
            Path: /auth/{proxy+}
            Method: ANY
```

### Function URL

No API Gateway needed — attach a Function URL directly to the Lambda function. Set `basePath: ""` in the handler options since Function URLs don't add a path prefix.
