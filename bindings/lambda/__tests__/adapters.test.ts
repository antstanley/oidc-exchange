import { describe, it, expect } from "vitest";
import type { APIGatewayProxyEvent, APIGatewayProxyEventV2, ALBEvent } from "aws-lambda";
import {
  isApiGatewayV1,
  isApiGatewayV2,
  isAlbEvent,
  fromApiGatewayV1,
  fromApiGatewayV2,
  fromAlbEvent,
} from "../src/adapters";

// ---------------------------------------------------------------------------
// Event detection
// ---------------------------------------------------------------------------

describe("event detection", () => {
  it("detects API Gateway v1 events", () => {
    const event = {
      httpMethod: "GET",
      resource: "/auth/{proxy+}",
      path: "/auth/health",
      headers: {},
      queryStringParameters: null,
      body: null,
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEvent;

    expect(isApiGatewayV1(event)).toBe(true);
    expect(isApiGatewayV2(event)).toBe(false);
    expect(isAlbEvent(event)).toBe(false);
  });

  it("detects API Gateway v2 events", () => {
    const event = {
      version: "2.0",
      requestContext: {
        http: { method: "GET", path: "/auth/health" },
      },
      headers: {},
      rawPath: "/auth/health",
      rawQueryString: "",
      body: null,
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEventV2;

    expect(isApiGatewayV2(event)).toBe(true);
    expect(isApiGatewayV1(event)).toBe(false);
    expect(isAlbEvent(event)).toBe(false);
  });

  it("detects ALB events", () => {
    const event = {
      httpMethod: "GET",
      path: "/auth/health",
      headers: {},
      requestContext: { elb: { targetGroupArn: "arn:..." } },
      queryStringParameters: null,
      body: null,
      isBase64Encoded: false,
    } as unknown as ALBEvent;

    expect(isAlbEvent(event)).toBe(true);
    expect(isApiGatewayV1(event)).toBe(false);
    expect(isApiGatewayV2(event)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// API Gateway v1 adapter
// ---------------------------------------------------------------------------

describe("fromApiGatewayV1", () => {
  it("strips basePath from path", () => {
    const event = {
      httpMethod: "POST",
      path: "/auth/token",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      multiValueHeaders: null,
      queryStringParameters: null,
      body: "grant_type=authorization_code&code=abc",
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEvent;

    const req = fromApiGatewayV1(event, "/auth");
    expect(req.method).toBe("POST");
    expect(req.path).toBe("/token");
    expect(req.headers).toContainEqual({
      name: "content-type",
      value: "application/x-www-form-urlencoded",
    });
    expect(req.body?.toString("utf-8")).toBe("grant_type=authorization_code&code=abc");
  });

  it("preserves path when no basePath", () => {
    const event = {
      httpMethod: "GET",
      path: "/health",
      headers: {},
      multiValueHeaders: null,
      queryStringParameters: null,
      body: null,
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEvent;

    const req = fromApiGatewayV1(event, "");
    expect(req.path).toBe("/health");
  });

  it("includes query string parameters", () => {
    const event = {
      httpMethod: "GET",
      path: "/health",
      headers: {},
      multiValueHeaders: null,
      queryStringParameters: { debug: "true" },
      body: null,
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEvent;

    const req = fromApiGatewayV1(event, "");
    expect(req.path).toBe("/health?debug=true");
  });

  it("decodes base64 body", () => {
    const bodyText = "grant_type=refresh_token&refresh_token=xyz";
    const event = {
      httpMethod: "POST",
      path: "/token",
      headers: {},
      multiValueHeaders: null,
      queryStringParameters: null,
      body: Buffer.from(bodyText).toString("base64"),
      isBase64Encoded: true,
    } as unknown as APIGatewayProxyEvent;

    const req = fromApiGatewayV1(event, "");
    expect(req.body?.toString("utf-8")).toBe(bodyText);
  });

  it("uses multiValueHeaders when present", () => {
    const event = {
      httpMethod: "GET",
      path: "/health",
      headers: { accept: "text/html" },
      multiValueHeaders: {
        accept: ["text/html", "application/json"],
      },
      queryStringParameters: null,
      body: null,
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEvent;

    const req = fromApiGatewayV1(event, "");
    const acceptHeaders = req.headers.filter((h) => h.name === "accept");
    expect(acceptHeaders).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// API Gateway v2 adapter
// ---------------------------------------------------------------------------

describe("fromApiGatewayV2", () => {
  it("strips basePath and includes query string", () => {
    const event = {
      version: "2.0",
      requestContext: {
        http: { method: "GET", path: "/auth/keys" },
      },
      headers: {},
      rawPath: "/auth/keys",
      rawQueryString: "format=jwks",
      body: null,
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEventV2;

    const req = fromApiGatewayV2(event, "/auth");
    expect(req.method).toBe("GET");
    expect(req.path).toBe("/keys?format=jwks");
  });

  it("handles Function URL events (same as v2)", () => {
    const event = {
      version: "2.0",
      requestContext: {
        http: { method: "POST", path: "/token" },
      },
      headers: { "content-type": "application/x-www-form-urlencoded" },
      rawPath: "/token",
      rawQueryString: "",
      body: "grant_type=authorization_code",
      isBase64Encoded: false,
    } as unknown as APIGatewayProxyEventV2;

    const req = fromApiGatewayV2(event, "");
    expect(req.method).toBe("POST");
    expect(req.path).toBe("/token");
    expect(req.body?.toString("utf-8")).toBe("grant_type=authorization_code");
  });
});

// ---------------------------------------------------------------------------
// ALB adapter
// ---------------------------------------------------------------------------

describe("fromAlbEvent", () => {
  it("strips basePath from ALB path", () => {
    const event = {
      httpMethod: "GET",
      path: "/auth/.well-known/openid-configuration",
      headers: {},
      multiValueHeaders: null,
      requestContext: { elb: { targetGroupArn: "arn:..." } },
      queryStringParameters: null,
      body: null,
      isBase64Encoded: false,
    } as unknown as ALBEvent;

    const req = fromAlbEvent(event, "/auth");
    expect(req.method).toBe("GET");
    expect(req.path).toBe("/.well-known/openid-configuration");
  });
});
