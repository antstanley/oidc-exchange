export interface HeaderEntry {
  name: string;
  value: string;
}

export interface HttpRequest {
  method: string;
  /** Raw path bytes only; never include the query string. */
  rawPath: Buffer;
  /** Raw query bytes without the leading question mark. */
  query?: Buffer;
  headers: HeaderEntry[];
  body?: Buffer;
  /** True only when rawPath came from a host-provided raw request target. */
  pathIsRaw: boolean;
}

export interface HttpResponse {
  status: number;
  headers: HeaderEntry[];
  body: Buffer;
}

export interface Limits {
  maxBodyBytes: number;
}

export interface OidcExchangeOptions {
  config?: string;
  configString?: string;
  /** Validated override for server.base_path, applied before router construction. */
  basePath?: string;
}

export class OidcExchange {
  constructor(options: OidcExchangeOptions);
  handleRequest(request: HttpRequest): Promise<HttpResponse>;
  /** @deprecated Await handleRequest instead. */
  handleRequestSync(request: HttpRequest): HttpResponse;
  limits(): Limits;
  shutdown(): void;
}
