export interface HeaderEntry {
  name: string;
  value: string;
}

export interface HttpRequest {
  method: string;
  path: string;
  headers: HeaderEntry[];
  body?: Buffer;
}

export interface HttpResponse {
  status: number;
  headers: HeaderEntry[];
  body: Buffer;
}

export interface OidcExchangeOptions {
  /** Path to a TOML configuration file. */
  config?: string;
  /** Inline TOML configuration string. */
  configString?: string;
}

export class OidcExchange {
  constructor(options: OidcExchangeOptions);

  /** Route an HTTP request through the embedded OIDC-Exchange server. */
  handleRequest(request: HttpRequest): HttpResponse;

  /** Graceful shutdown (currently a no-op). */
  shutdown(): void;
}
