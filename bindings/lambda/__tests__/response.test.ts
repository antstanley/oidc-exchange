import { describe, expect, it } from "vitest";
import type { APIGatewayProxyEvent, APIGatewayProxyEventV2, ALBEvent } from "aws-lambda";
import { translateResponse } from "../src/index";

const response = {
  status: 201,
  headers: [
    { name: "set-cookie", value: "a=1" },
    { name: "x-repeat", value: "first" },
    { name: "Set-Cookie", value: "b=2" },
    { name: "x-repeat", value: "second" },
    { name: "content-type", value: "text/plain" },
  ],
  body: Buffer.from("ok"),
};
const v2 = { version: "2.0", requestContext: { http: {} } } as unknown as APIGatewayProxyEventV2;
const v1 = { httpMethod: "GET", requestContext: {} } as unknown as APIGatewayProxyEvent;
const alb = { httpMethod: "GET", requestContext: { elb: {} } } as unknown as ALBEvent;

describe("translateResponse", () => {
  it("emits every v2 cookie separately and preserves representable ordinary values", () => {
    expect(translateResponse(v2, response)).toEqual({
      statusCode: 201,
      headers: {
        "x-repeat": "first, second",
        "content-type": "text/plain",
      },
      cookies: ["a=1", "b=2"],
      body: "b2s=",
      isBase64Encoded: true,
    });
  });
  it.each([
    ["API Gateway v1", v1],
    ["ALB", alb],
  ])("emits ordered multi-value headers for %s", (_name, event) => {
    expect(translateResponse(event, response)).toEqual({
      statusCode: 201,
      headers: {
        "set-cookie": "a=1",
        "x-repeat": "first",
        "Set-Cookie": "b=2",
        "content-type": "text/plain",
      },
      multiValueHeaders: {
        "set-cookie": ["a=1"],
        "x-repeat": ["first", "second"],
        "Set-Cookie": ["b=2"],
        "content-type": ["text/plain"],
      },
      body: "b2s=",
      isBase64Encoded: true,
    });
  });
});
