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
  isAlbEvent,
  isApiGatewayV1,
  isApiGatewayV2,
} from "./adapters";
import type { LambdaHandlerOptions } from "./types";

export type { LambdaHandlerOptions } from "./types";

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
  const { basePath = "", ...oidcOptions } = options;
  const oidc = new OidcExchange(oidcOptions);

  return async (event: LambdaEvent, _context: Context): Promise<LambdaResult> => {
    const request = normalise(event, basePath);

    const response = oidc.handleRequest({
      method: request.method,
      path: request.path,
      headers: request.headers,
      body: request.body,
    });

    // Build response headers as a plain object
    const responseHeaders: Record<string, string> = {};
    for (const { name, value } of response.headers) {
      responseHeaders[name] = value;
    }

    const bodyBase64 = Buffer.from(response.body).toString("base64");

    // Return the right shape for the event source
    if (isApiGatewayV2(event)) {
      return {
        statusCode: response.status,
        headers: responseHeaders,
        body: bodyBase64,
        isBase64Encoded: true,
      } satisfies APIGatewayProxyResultV2;
    }

    // v1 and ALB use the same response shape
    return {
      statusCode: response.status,
      headers: responseHeaders,
      body: bodyBase64,
      isBase64Encoded: true,
    } satisfies APIGatewayProxyResult;
  };
}

function normalise(event: LambdaEvent, basePath: string) {
  if (isApiGatewayV2(event)) return fromApiGatewayV2(event, basePath);
  if (isAlbEvent(event)) return fromAlbEvent(event, basePath);
  if (isApiGatewayV1(event)) return fromApiGatewayV1(event, basePath);

  // Fallback — treat as v1-like
  return fromApiGatewayV1(event as APIGatewayProxyEvent, basePath);
}
