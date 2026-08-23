import type { APIGatewayProxyEvent, APIGatewayProxyEventV2, ALBEvent } from "aws-lambda";
import type { HttpRequest, HeaderEntry } from "@oidc-exchange/node";

export class BodyTooLargeError extends Error {}
export type NormalisedRequest = HttpRequest;

export function isApiGatewayV1(event: unknown): event is APIGatewayProxyEvent {
  const e = event as Record<string, unknown>;
  return typeof e.httpMethod === "string" && typeof e.resource === "string" && !("version" in e);
}
export function isApiGatewayV2(event: unknown): event is APIGatewayProxyEventV2 {
  const e = event as Record<string, unknown>;
  return e.version === "2.0" && typeof e.requestContext === "object";
}
export function isAlbEvent(event: unknown): event is ALBEvent {
  const e = event as Record<string, unknown>;
  return typeof e.httpMethod === "string" && typeof e.requestContext === "object" && (e.requestContext as Record<string, unknown>).elb !== undefined;
}

export function fromApiGatewayV1(event: APIGatewayProxyEvent, maxBodyBytes = Number.MAX_SAFE_INTEGER): NormalisedRequest {
  return {
    method: event.httpMethod,
    rawPath: Buffer.from(event.path || "/"),
    query: encodeQuery(event.multiValueQueryStringParameters, event.queryStringParameters),
    headers: flattenHeaders(event.headers, event.multiValueHeaders),
    body: decodeBody(event.body, event.isBase64Encoded, maxBodyBytes),
    pathIsRaw: false,
  };
}

export function fromApiGatewayV2(event: APIGatewayProxyEventV2, maxBodyBytes = Number.MAX_SAFE_INTEGER): NormalisedRequest {
  return {
    method: event.requestContext?.http?.method ?? "GET",
    rawPath: Buffer.from(event.rawPath || event.requestContext?.http?.path || "/"),
    query: event.rawQueryString ? Buffer.from(event.rawQueryString) : undefined,
    headers: flattenV2Headers(event.headers, event.cookies),
    body: decodeBody(event.body, event.isBase64Encoded, maxBodyBytes),
    pathIsRaw: Boolean(event.rawPath),
  };
}

export function fromAlbEvent(event: ALBEvent, maxBodyBytes = Number.MAX_SAFE_INTEGER): NormalisedRequest {
  return {
    method: event.httpMethod,
    rawPath: Buffer.from(event.path || "/"),
    query: encodeQuery(event.multiValueQueryStringParameters, event.queryStringParameters),
    headers: flattenHeaders(event.headers, event.multiValueHeaders),
    body: decodeBody(event.body, event.isBase64Encoded, maxBodyBytes),
    pathIsRaw: false,
  };
}

function flattenHeaders(single?: Record<string, string | undefined> | null, multi?: Record<string, string[] | undefined> | null): HeaderEntry[] {
  const headers: HeaderEntry[] = [];
  if (multi) for (const [name, values] of Object.entries(multi)) for (const value of values ?? []) headers.push({ name, value });
  else if (single) for (const [name, value] of Object.entries(single)) if (value !== undefined) headers.push({ name, value });
  return headers;
}

function flattenV2Headers(single?: Record<string, string | undefined> | null, cookies?: string[]): HeaderEntry[] {
  const headers = flattenHeaders(single);
  for (const value of cookies ?? []) headers.push({ name: "cookie", value });
  return headers;
}

function encodeQuery(multi?: Record<string, string[] | undefined> | null, single?: Record<string, string | undefined> | null): Buffer | undefined {
  const pairs: string[] = [];
  if (multi) for (const [name, values] of Object.entries(multi)) for (const value of values ?? []) pairs.push(`${encodeURIComponent(name)}=${encodeURIComponent(value)}`);
  else if (single) for (const [name, value] of Object.entries(single)) if (value != null) pairs.push(`${encodeURIComponent(name)}=${encodeURIComponent(value)}`);
  return pairs.length ? Buffer.from(pairs.join("&")) : undefined;
}

function decodeBody(body: string | null | undefined, base64: boolean | undefined, limit: number): Buffer | undefined {
  if (!body) return undefined;
  if (base64 && Math.floor(body.length / 4) * 3 > limit + 2) throw new BodyTooLargeError();
  if (!base64 && Buffer.byteLength(body, "utf8") > limit) throw new BodyTooLargeError();
  const decoded = Buffer.from(body, base64 ? "base64" : "utf8");
  if (decoded.length > limit) throw new BodyTooLargeError();
  return decoded;
}
