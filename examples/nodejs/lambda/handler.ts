import { createHandler } from "@oidc-exchange/lambda";

/**
 * Lambda handler for oidc-exchange.
 *
 * Automatically detects the event source:
 * - API Gateway REST API (v1)
 * - API Gateway HTTP API (v2)
 * - Lambda Function URL
 * - Application Load Balancer (ALB)
 *
 * Deploy with any of these triggers and it just works.
 */
export const handler = createHandler({
  config: "./config.toml",
  basePath: "/auth",
});
