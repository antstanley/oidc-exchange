import type { OidcExchangeOptions } from "@oidc-exchange/node";

/** Options for creating a Lambda handler. */
export interface LambdaHandlerOptions extends OidcExchangeOptions {
  /**
   * Deployment base path. Translation never strips it; configure the embedded
   * service consistently so the shared normaliser owns path handling.
   */
  basePath?: string;
}
