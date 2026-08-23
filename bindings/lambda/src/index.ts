import type {
  APIGatewayProxyEvent,
  APIGatewayProxyEventV2,
  APIGatewayProxyResult,
  APIGatewayProxyResultV2,
  ALBEvent,
  ALBResult,
  Context,
} from "aws-lambda";
import { OidcExchange } from "@oidc-exchange/node";

import {
  fromAlbEvent,
  fromApiGatewayV1,
  fromApiGatewayV2,
  BodyTooLargeError,
  isAlbEvent,
  isApiGatewayV1,
  isApiGatewayV2,
} from "./adapters.js";
import type { LambdaHandlerOptions } from "./types.js";

export type { LambdaHandlerOptions } from "./types.js";

type LambdaEvent = APIGatewayProxyEvent | APIGatewayProxyEventV2 | ALBEvent;
type LambdaResult = APIGatewayProxyResult | APIGatewayProxyResultV2 | ALBResult;

/**
 * Create a Lambda handler that routes HTTP events through oidc-exchange.
 *
 * Automatically detects the event source (API Gateway v1, API Gateway v2 /
 * Function URL, or ALB) and translates the event into an HTTP request.
 *
 * @example API Gateway v2 / Function URL
 * ```ts
 * import { createHandler } from "@oidc-exchange/lambda";
 *
 * export const handler = createHandler({
 *   config: "./config.toml",
 *   basePath: "/auth",
 * });
 * ```
 *
 * @example Inline configuration
 * ```ts
 * export const handler = createHandler({
 *   configString: `
 *     [server]
 *     issuer = "https://auth.example.com"
 *     role = "exchange"
 *     ...
 *   `,
 * });
 * ```
 */
export function createHandler(
  options: LambdaHandlerOptions,
): (event: LambdaEvent, context: Context) => Promise<LambdaResult> {
  const oidc = new OidcExchange(options);
  const maxBodyBytes = oidc.limits().maxBodyBytes;

  return async (event: LambdaEvent, _context: Context): Promise<LambdaResult> => {
    let request;
    try {
      request = normalise(event, maxBodyBytes);
    } catch (error) {
      if (error instanceof BodyTooLargeError) {
        return { statusCode: 413, body: "", isBase64Encoded: false };
      }
      throw error;
    }

    const response = await oidc.handleRequest(request);

    return translateResponse(event, response);
  };
}

function normalise(event: LambdaEvent, maxBodyBytes: number) {
  if (isApiGatewayV2(event)) return fromApiGatewayV2(event, maxBodyBytes);
  if (isAlbEvent(event)) return fromAlbEvent(event, maxBodyBytes);
  if (isApiGatewayV1(event)) return fromApiGatewayV1(event, maxBodyBytes);
  return fromApiGatewayV1(event as APIGatewayProxyEvent, maxBodyBytes);
}

export function translateResponse(
  event: LambdaEvent,
  response: { status: number; headers: Array<{ name: string; value: string }>; body: Uint8Array },
): LambdaResult {
  const body = Buffer.from(response.body).toString("base64");
  if (isApiGatewayV2(event)) {
    const headers: Record<string, string> = {};
    const cookies: string[] = [];
    for (const { name, value } of response.headers) {
      if (name.toLowerCase() === "set-cookie") cookies.push(value);
      else headers[name] = name in headers ? `${headers[name]}, ${value}` : value;
    }
    return {
      statusCode: response.status,
      headers,
      ...(cookies.length ? { cookies } : {}),
      body,
      isBase64Encoded: true,
    };
  }
  const headers: Record<string, string> = {};
  const multiValueHeaders: Record<string, string[]> = {};
  for (const { name, value } of response.headers) {
    (multiValueHeaders[name] ??= []).push(value);
    if (!(name in headers)) headers[name] = value;
  }
  return { statusCode: response.status, headers, multiValueHeaders, body, isBase64Encoded: true };
}
