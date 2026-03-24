import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as kms from 'aws-cdk-lib/aws-kms';
import * as cloudtrail from 'aws-cdk-lib/aws-cloudtrail';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as apigwv2 from 'aws-cdk-lib/aws-apigatewayv2';
import * as iam from 'aws-cdk-lib/aws-iam';
import { Construct } from 'constructs';
import * as path from 'path';

interface OidcExchangeExampleStackProps extends cdk.StackProps {
  googleClientId: string;
  googleClientSecret: string;
}

export class OidcExchangeExampleStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: OidcExchangeExampleStackProps) {
    super(scope, id, props);

    // ── DynamoDB Table ──────────────────────────────────────────────
    const table = new dynamodb.Table(this, 'Table', {
      tableName: 'oidc-exchange-example',
      partitionKey: { name: 'pk', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'sk', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: 'ttl',
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    table.addGlobalSecondaryIndex({
      indexName: 'GSI1',
      partitionKey: { name: 'GSI1pk', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'GSI1sk', type: dynamodb.AttributeType.STRING },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // ── KMS Key (ECC_NIST_P256 for signing) ─────────────────────────
    const key = new kms.Key(this, 'SigningKey', {
      keySpec: kms.KeySpec.ECC_NIST_P256,
      keyUsage: kms.KeyUsage.SIGN_VERIFY,
      alias: 'oidc-exchange-example',
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    // ── CloudTrail Lake (L1 constructs) ─────────────────────────────
    const eventDataStore = new cloudtrail.CfnEventDataStore(this, 'AuditEventDataStore', {
      name: 'oidc-exchange-example-audit',
      retentionPeriod: 30,
      advancedEventSelectors: [
        {
          name: 'CustomAuditEvents',
          fieldSelectors: [
            {
              field: 'eventCategory',
              equalTo: ['ActivityAuditLog'],
            },
          ],
        },
      ],
    });

    const channel = new cloudtrail.CfnChannel(this, 'AuditChannel', {
      name: 'oidc-exchange-example-audit',
      source: 'Custom',
      destinations: [
        {
          type: 'EVENT_DATA_STORE',
          location: eventDataStore.attrEventDataStoreArn,
        },
      ],
    });

    // ── Auth Lambda (Rust / provided.al2023) ────────────────────────
    const authFunction = new lambda.Function(this, 'AuthFunction', {
      runtime: lambda.Runtime.PROVIDED_AL2023,
      handler: 'bootstrap',
      code: lambda.Code.fromAsset(path.join(__dirname, '../../bootstrap')),
      memorySize: 256,
      timeout: cdk.Duration.seconds(29),
      environment: {
        TABLE_NAME: table.tableName,
        KMS_KEY_ID: key.keyArn,
        CLOUDTRAIL_CHANNEL_ARN: channel.attrChannelArn,
        GOOGLE_CLIENT_ID: props.googleClientId,
        GOOGLE_CLIENT_SECRET: props.googleClientSecret,
        // ISSUER_URL and AUDIENCE_URL are set after API Gateway creation
        ISSUER_URL: 'PLACEHOLDER',
        AUDIENCE_URL: 'PLACEHOLDER',
      },
    });

    table.grantReadWriteData(authFunction);
    key.grant(authFunction, 'kms:Sign', 'kms:GetPublicKey');

    authFunction.addToRolePolicy(
      new iam.PolicyStatement({
        actions: ['cloudtrail-data:PutAuditEvents'],
        resources: [channel.attrChannelArn],
      })
    );

    // ── Demo App Lambda (Node.js / SvelteKit) ───────────────────────
    const webAdapterLayer = lambda.LayerVersion.fromLayerVersionArn(
      this,
      'WebAdapterLayer',
      `arn:aws:lambda:${cdk.Stack.of(this).region}:753240598075:layer:LambdaAdapterLayerX86:24`
    );

    const demoAppFunction = new lambda.Function(this, 'DemoAppFunction', {
      runtime: lambda.Runtime.NODEJS_22_X,
      handler: 'run.sh',
      code: lambda.Code.fromAsset(path.join(__dirname, '../../demo-app/dist/svelteKit')),
      memorySize: 256,
      timeout: cdk.Duration.seconds(29),
      layers: [webAdapterLayer],
      environment: {
        AWS_LAMBDA_EXEC_WRAPPER: '/opt/bootstrap',
        PORT: '8080',
        // ORIGIN and AUTH_ENDPOINT are set after API Gateway creation
        ORIGIN: 'PLACEHOLDER',
        AUTH_ENDPOINT: 'PLACEHOLDER',
        PUBLIC_GOOGLE_CLIENT_ID: props.googleClientId,
      },
    });

    // ── API Gateway (HTTP API via L1 constructs) ────────────────────
    const httpApi = new apigwv2.CfnApi(this, 'HttpApi', {
      name: 'OidcExchangeExampleApi',
      protocolType: 'HTTP',
    });

    const stage = new apigwv2.CfnStage(this, 'DefaultStage', {
      apiId: httpApi.ref,
      stageName: '$default',
      autoDeploy: true,
    });

    // Auth Lambda integration
    const authIntegration = new apigwv2.CfnIntegration(this, 'AuthIntegration', {
      apiId: httpApi.ref,
      integrationType: 'AWS_PROXY',
      integrationUri: authFunction.functionArn,
      payloadFormatVersion: '2.0',
    });

    new apigwv2.CfnRoute(this, 'AuthRoute', {
      apiId: httpApi.ref,
      routeKey: 'ANY /auth/{proxy+}',
      target: `integrations/${authIntegration.ref}`,
    });

    // Demo App Lambda integration
    const demoIntegration = new apigwv2.CfnIntegration(this, 'DemoIntegration', {
      apiId: httpApi.ref,
      integrationType: 'AWS_PROXY',
      integrationUri: demoAppFunction.functionArn,
      payloadFormatVersion: '2.0',
    });

    new apigwv2.CfnRoute(this, 'DemoProxyRoute', {
      apiId: httpApi.ref,
      routeKey: 'ANY /{proxy+}',
      target: `integrations/${demoIntegration.ref}`,
    });

    new apigwv2.CfnRoute(this, 'DemoDefaultRoute', {
      apiId: httpApi.ref,
      routeKey: '$default',
      target: `integrations/${demoIntegration.ref}`,
    });

    // Grant API Gateway permission to invoke Lambdas
    authFunction.addPermission('ApiGatewayInvoke', {
      principal: new iam.ServicePrincipal('apigateway.amazonaws.com'),
      sourceArn: `arn:aws:execute-api:${this.region}:${this.account}:${httpApi.ref}/*`,
    });

    demoAppFunction.addPermission('ApiGatewayInvoke', {
      principal: new iam.ServicePrincipal('apigateway.amazonaws.com'),
      sourceArn: `arn:aws:execute-api:${this.region}:${this.account}:${httpApi.ref}/*`,
    });

    // Construct API URL and update Lambda environments
    const apiUrl = `https://${httpApi.ref}.execute-api.${this.region}.amazonaws.com`;

    // Update Auth Lambda environment with actual URLs
    const cfnAuthFunction = authFunction.node.defaultChild as lambda.CfnFunction;
    cfnAuthFunction.addPropertyOverride('Environment.Variables.ISSUER_URL', `${apiUrl}/auth`);
    cfnAuthFunction.addPropertyOverride('Environment.Variables.AUDIENCE_URL', apiUrl);

    // Update Demo App Lambda environment with actual URLs
    const cfnDemoFunction = demoAppFunction.node.defaultChild as lambda.CfnFunction;
    cfnDemoFunction.addPropertyOverride('Environment.Variables.ORIGIN', apiUrl);
    cfnDemoFunction.addPropertyOverride('Environment.Variables.AUTH_ENDPOINT', `${apiUrl}/auth`);

    // ── Stack Outputs ───────────────────────────────────────────────
    new cdk.CfnOutput(this, 'ApiUrl', {
      value: apiUrl,
      description: 'HTTP API Gateway URL',
    });

    new cdk.CfnOutput(this, 'TableName', {
      value: table.tableName,
      description: 'DynamoDB table name',
    });

    new cdk.CfnOutput(this, 'KmsKeyArn', {
      value: key.keyArn,
      description: 'KMS signing key ARN',
    });
  }
}
