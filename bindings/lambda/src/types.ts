import type { OidcExchangeOptions } from "@oidc-exchange/node";

/** Options for creating a Lambda handler. */
export interface LambdaHandlerOptions extends OidcExchangeOptions {
  /**
   * Validated override for `server.base_path`. It is applied to the typed
   * service configuration before the FFI router is constructed.
   */
  basePath?: string;
}
