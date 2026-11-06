import { CfnOutput, Duration, RemovalPolicy, Stack, type StackProps } from "aws-cdk-lib";
import * as acm from "aws-cdk-lib/aws-certificatemanager";
import * as cloudfront from "aws-cdk-lib/aws-cloudfront";
import * as origins from "aws-cdk-lib/aws-cloudfront-origins";
import * as iam from "aws-cdk-lib/aws-iam";
import * as route53 from "aws-cdk-lib/aws-route53";
import * as targets from "aws-cdk-lib/aws-route53-targets";
import * as s3 from "aws-cdk-lib/aws-s3";
import type { Construct } from "constructs";

export const ACCOUNT = "115019372186";
export const REGION = "us-east-1";
const DOMAIN_NAME = "dl.oidc-exchange.iamstan.dev";
export const BUCKET_NAME = `oidc-exchange-downloads-${ACCOUNT}`;
const UPLOAD_ROLE_NAME = "oidc-exchange-downloads-upload";
const REPO = "antstanley/oidc-exchange";
const OIDC_HOST = "token.actions.githubusercontent.com";

export interface DownloadsStackProps extends StackProps {
  /** Hosted zone that contains the download hostname (lives in this account). */
  hostedZoneId: string;
  zoneName: string;
}

export class DownloadsStack extends Stack {
  constructor(scope: Construct, id: string, props: DownloadsStackProps) {
    super(scope, id, props);

    const zone = route53.HostedZone.fromHostedZoneAttributes(this, "Zone", {
      hostedZoneId: props.hostedZoneId,
      zoneName: props.zoneName,
    });

    const bucket = new s3.Bucket(this, "Bucket", {
      bucketName: BUCKET_NAME,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      enforceSSL: true,
      versioned: true,
      encryption: s3.BucketEncryption.S3_MANAGED,
      objectOwnership: s3.ObjectOwnership.BUCKET_OWNER_ENFORCED,
      removalPolicy: RemovalPolicy.RETAIN,
    });

    const certificate = new acm.Certificate(this, "Certificate", {
      domainName: DOMAIN_NAME,
      validation: acm.CertificateValidation.fromDns(zone),
    });

    const headers = new cloudfront.ResponseHeadersPolicy(this, "Headers", {
      responseHeadersPolicyName: "oidc-exchange-downloads-security",
      securityHeadersBehavior: {
        strictTransportSecurity: {
          accessControlMaxAge: Duration.days(365),
          includeSubdomains: true,
          override: true,
        },
        contentTypeOptions: { override: true },
      },
    });

    const distribution = new cloudfront.Distribution(this, "Distribution", {
      comment: "oidc-exchange-downloads",
      domainNames: [DOMAIN_NAME],
      certificate,
      minimumProtocolVersion: cloudfront.SecurityPolicyProtocol.TLS_V1_2_2021,
      httpVersion: cloudfront.HttpVersion.HTTP2_AND_3,
      defaultBehavior: {
        origin: origins.S3BucketOrigin.withOriginAccessControl(bucket),
        viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
        allowedMethods: cloudfront.AllowedMethods.ALLOW_GET_HEAD,
        // Honours the Cache-Control the uploader sets: immutable versioned objects
        // for a year, latest.json for 60 seconds.
        cachePolicy: cloudfront.CachePolicy.CACHING_OPTIMIZED,
        responseHeadersPolicy: headers,
        compress: false,
      },
    });

    const recordTarget = route53.RecordTarget.fromAlias(new targets.CloudFrontTarget(distribution));
    new route53.ARecord(this, "Alias", { zone, recordName: DOMAIN_NAME, target: recordTarget });
    new route53.AaaaRecord(this, "AliasIpv6", {
      zone,
      recordName: DOMAIN_NAME,
      target: recordTarget,
    });

    // The account already holds the GitHub OIDC provider (blogwright bootstrap);
    // AWS rejects a second provider for the same URL, so import it.
    const provider = iam.OpenIdConnectProvider.fromOpenIdConnectProviderArn(
      this,
      "GithubProvider",
      `arn:aws:iam::${ACCOUNT}:oidc-provider/${OIDC_HOST}`,
    );

    const role = new iam.Role(this, "UploadRole", {
      roleName: UPLOAD_ROLE_NAME,
      description: "Release workflow upload to the oidc-exchange download bucket",
      maxSessionDuration: Duration.hours(1),
      assumedBy: new iam.WebIdentityPrincipal(provider.openIdConnectProviderArn, {
        StringEquals: {
          [`${OIDC_HOST}:aud`]: "sts.amazonaws.com",
          [`${OIDC_HOST}:sub`]: `repo:${REPO}:environment:downloads`,
        },
        StringLike: {
          [`${OIDC_HOST}:job_workflow_ref`]: `${REPO}/.github/workflows/release.yml@refs/tags/v*.*.*`,
        },
      }),
    });
    // PutObject only: no delete, no list, no CloudFront or IAM access.
    role.addToPolicy(
      new iam.PolicyStatement({
        actions: ["s3:PutObject"],
        resources: [
          bucket.arnForObjects("releases/*"),
          bucket.arnForObjects("latest.json"),
          bucket.arnForObjects("install.sh"),
          bucket.arnForObjects("install.sh.sigstore.bundle"),
        ],
      }),
    );

    new CfnOutput(this, "DistributionDomain", { value: distribution.distributionDomainName });
    new CfnOutput(this, "UploadRoleArn", { value: role.roleArn });
    new CfnOutput(this, "BucketName", { value: bucket.bucketName });
  }
}
