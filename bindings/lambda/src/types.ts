import type { OidcExchangeOptions } from "@oidc-exchange/node";

/** Options for creating a Lambda handler. */
export interface LambdaHandlerOptions extends OidcExchangeOptions {
  /**
   * Base path prefix to strip from incoming requests.
   *
   * For example, if your API Gateway routes `/auth/{proxy+}` to this handler,
   * set `basePath` to `"/auth"` so that `/auth/token` becomes `/token`.
   *
   * @default "" (no stripping)
   */
  basePath?: string;
}
