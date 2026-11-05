import { App } from "aws-cdk-lib";
import { Match, Template } from "aws-cdk-lib/assertions";
import { describe, expect, it } from "vitest";
import { ACCOUNT, BUCKET_NAME, DownloadsStack, REGION } from "../lib/downloads-stack.ts";

const template = Template.fromStack(
  new DownloadsStack(new App(), "Test", {
    env: { account: ACCOUNT, region: REGION },
    hostedZoneId: "Z0000000000TEST",
    zoneName: "iamstan.dev",
  }),
);

describe("DownloadsStack", () => {
  it("keeps the bucket private, versioned, TLS-only and retained", () => {
    template.hasResourceProperties("AWS::S3::Bucket", {
      BucketName: BUCKET_NAME,
      PublicAccessBlockConfiguration: {
        BlockPublicAcls: true,
        BlockPublicPolicy: true,
        IgnorePublicAcls: true,
        RestrictPublicBuckets: true,
      },
      VersioningConfiguration: { Status: "Enabled" },
    });
    template.hasResource("AWS::S3::Bucket", {
      DeletionPolicy: "Retain",
      UpdateReplacePolicy: "Retain",
    });
    template.hasResourceProperties("AWS::S3::BucketPolicy", {
      PolicyDocument: {
        Statement: Match.arrayWith([
          Match.objectLike({
            Effect: "Deny",
            Condition: { Bool: { "aws:SecureTransport": "false" } },
          }),
        ]),
      },
    });
  });

  it("serves through CloudFront with OAC, HTTPS redirect and security headers", () => {
    template.resourceCountIs("AWS::CloudFront::OriginAccessControl", 1);
    template.hasResourceProperties("AWS::CloudFront::Distribution", {
      DistributionConfig: Match.objectLike({
        Aliases: ["dl.oidc-exchange.iamstan.dev"],
        DefaultCacheBehavior: Match.objectLike({ ViewerProtocolPolicy: "redirect-to-https" }),
      }),
    });
    template.hasResourceProperties("AWS::CloudFront::ResponseHeadersPolicy", {
      ResponseHeadersPolicyConfig: Match.objectLike({
        SecurityHeadersConfig: Match.objectLike({
          StrictTransportSecurity: Match.objectLike({ Override: true }),
          ContentTypeOptions: { Override: true },
        }),
      }),
    });
  });

  it("points the hostname at the distribution", () => {
    template.hasResourceProperties("AWS::Route53::RecordSet", {
      Name: "dl.oidc-exchange.iamstan.dev.",
      Type: "A",
    });
  });

  it("scopes the upload role to the release workflow and PutObject only", () => {
    template.hasResourceProperties("AWS::IAM::Role", {
      RoleName: "oidc-exchange-downloads-upload",
      AssumeRolePolicyDocument: {
        Statement: [
          Match.objectLike({
            Action: "sts:AssumeRoleWithWebIdentity",
            Condition: {
              StringEquals: {
                "token.actions.githubusercontent.com:aud": "sts.amazonaws.com",
                "token.actions.githubusercontent.com:sub":
                  "repo:antstanley/oidc-exchange:environment:downloads",
              },
              StringLike: {
                "token.actions.githubusercontent.com:job_workflow_ref":
                  "antstanley/oidc-exchange/.github/workflows/release.yml@refs/tags/v*.*.*",
              },
            },
          }),
        ],
      },
    });
    const policies = template.findResources("AWS::IAM::Policy");
    const statements = Object.values(policies).flatMap(
      (p) =>
        (p as { Properties: { PolicyDocument: { Statement: unknown[] } } }).Properties
          .PolicyDocument.Statement,
    ) as { Action: string | string[] }[];
    expect(statements.map((s) => s.Action)).toEqual(["s3:PutObject"]);
  });
});
