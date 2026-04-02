import type { APIGatewayProxyEvent, APIGatewayProxyEventV2, ALBEvent } from "aws-lambda";
import type { HeaderEntry } from "@oidc-exchange/node";

/**
 * Normalised request extracted from any supported Lambda event type.
 */
export interface NormalisedRequest {
  method: string;
  path: string;
  headers: HeaderEntry[];
  body?: Buffer;
}

// ---------------------------------------------------------------------------
// API Gateway REST API (v1)
// ---------------------------------------------------------------------------

export function isApiGatewayV1(event: unknown): event is APIGatewayProxyEvent {
  const e = event as Record<string, unknown>;
  return typeof e.httpMethod === "string" && typeof e.resource === "string" && !("version" in e);
}

export function fromApiGatewayV1(event: APIGatewayProxyEvent, basePath: string): NormalisedRequest {
  let path = event.path || "/";
  if (basePath && path.startsWith(basePath)) {
    path = path.slice(basePath.length) || "/";
  }

  if (event.queryStringParameters) {
    const qs = Object.entries(event.queryStringParameters)
      .filter(([, v]) => v != null)
      .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v!)}`)
      .join("&");
    if (qs) path = `${path}?${qs}`;
  }

  return {
    method: event.httpMethod,
    path,
    headers: flattenHeaders(event.headers, event.multiValueHeaders),
    body: decodeBody(event.body, event.isBase64Encoded),
  };
}

// ---------------------------------------------------------------------------
// API Gateway HTTP API (v2) / Function URL
// ---------------------------------------------------------------------------

export function isApiGatewayV2(event: unknown): event is APIGatewayProxyEventV2 {
  const e = event as Record<string, unknown>;
  return e.version === "2.0" && typeof e.requestContext === "object";
}

export function fromApiGatewayV2(
  event: APIGatewayProxyEventV2,
  basePath: string,
): NormalisedRequest {
  const rc = event.requestContext?.http;
  const method = rc?.method ?? "GET";

  let path = rc?.path ?? event.rawPath ?? "/";
  if (basePath && path.startsWith(basePath)) {
    path = path.slice(basePath.length) || "/";
  }

  if (event.rawQueryString) {
    path = `${path}?${event.rawQueryString}`;
  }

  const headers: HeaderEntry[] = [];
  if (event.headers) {
    for (const [name, value] of Object.entries(event.headers)) {
      if (value !== undefined) {
        // v2 headers can be comma-joined multi-values
        headers.push({ name, value });
      }
    }
  }

  return {
    method,
    path,
    headers,
    body: decodeBody(event.body, event.isBase64Encoded),
  };
}

// ---------------------------------------------------------------------------
// Application Load Balancer
// ---------------------------------------------------------------------------

export function isAlbEvent(event: unknown): event is ALBEvent {
  const e = event as Record<string, unknown>;
  return (
    typeof e.httpMethod === "string" &&
    typeof e.requestContext === "object" &&
    (e.requestContext as Record<string, unknown>)?.elb !== undefined
  );
}

export function fromAlbEvent(event: ALBEvent, basePath: string): NormalisedRequest {
  let path = event.path || "/";
  if (basePath && path.startsWith(basePath)) {
    path = path.slice(basePath.length) || "/";
  }

  if (event.queryStringParameters) {
    const qs = Object.entries(event.queryStringParameters)
      .filter(([, v]) => v != null)
      .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v!)}`)
      .join("&");
    if (qs) path = `${path}?${qs}`;
  }

  return {
    method: event.httpMethod,
    path,
    headers: flattenHeaders(event.headers, event.multiValueHeaders),
    body: decodeBody(event.body, event.isBase64Encoded),
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function flattenHeaders(
  single?: Record<string, string | undefined> | null,
  multi?: Record<string, string[] | undefined> | null,
): HeaderEntry[] {
  const headers: HeaderEntry[] = [];

  if (multi) {
    for (const [name, values] of Object.entries(multi)) {
      if (values) {
        for (const value of values) {
          headers.push({ name, value });
        }
      }
    }
  } else if (single) {
    for (const [name, value] of Object.entries(single)) {
      if (value !== undefined) {
        headers.push({ name, value });
      }
    }
  }

  return headers;
}

function decodeBody(
  body: string | null | undefined,
  isBase64Encoded?: boolean,
): Buffer | undefined {
  if (!body) return undefined;
  return Buffer.from(body, isBase64Encoded ? "base64" : "utf-8");
}
