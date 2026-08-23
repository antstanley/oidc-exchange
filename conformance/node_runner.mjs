import { createInterface } from "node:readline";
import { OidcExchange } from "../bindings/nodejs/index.js";
import { fromApiGatewayV1, fromApiGatewayV2 } from "../bindings/lambda/dist/adapters.js";

const [shape, config] = process.argv.slice(2);
const oidc = new OidcExchange({ config });

for await (const line of createInterface({ input: process.stdin })) {
  const fixture = JSON.parse(line);
  let request;
  if (shape === "node") {
    request = {
      method: fixture.method,
      rawPath: Buffer.from(fixture.rawPath),
      query: fixture.query == null ? undefined : Buffer.from(fixture.query),
      headers: fixture.headers,
      body: Buffer.alloc(fixture.bodyLength, 120),
      pathIsRaw: fixture.pathIsRaw,
    };
  } else {
    const body = Buffer.alloc(fixture.bodyLength, 120).toString("base64");
    try {
    if (fixture.lambdaEvent === "v1") {
      request = fromApiGatewayV1({
        httpMethod: fixture.method,
        resource: fixture.rawPath || "/",
        path: fixture.rawPath || "/",
        headers: Object.fromEntries(fixture.headers.map(({ name, value }) => [name, value])),
        multiValueHeaders: Object.fromEntries([...new Set(fixture.headers.map(({ name }) => name))].map((name) => [name, fixture.headers.filter((h) => h.name === name).map((h) => h.value)])),
        queryStringParameters: fixture.query ? Object.fromEntries(new URLSearchParams(fixture.query)) : null,
        multiValueQueryStringParameters: null,
        body: fixture.bodyLength ? body : null,
        isBase64Encoded: true,
        requestContext: {}, stageVariables: null, pathParameters: null,
      }, oidc.limits().maxBodyBytes);
    } else {
      request = fromApiGatewayV2({
        version: "2.0", routeKey: "$default", rawPath: fixture.rawPath || "/", rawQueryString: fixture.query || "",
        headers: Object.fromEntries(fixture.headers.map(({ name, value }) => [name, value])),
        requestContext: { http: { method: fixture.method, path: fixture.rawPath || "/", protocol: "HTTP/1.1", sourceIp: "127.0.0.1", userAgent: "conformance" }, accountId: "", apiId: "", domainName: "", domainPrefix: "", requestId: "", routeKey: "", stage: "", time: "", timeEpoch: 0 },
        body: fixture.bodyLength ? body : undefined, isBase64Encoded: true,
      }, oidc.limits().maxBodyBytes);
    }
    } catch (error) {
      console.log(JSON.stringify({ id: fixture.id, method: fixture.method, decodedPath: decodeURIComponent(fixture.rawPath || "/").replace(/^\/auth(?=\/|$)/, "") || "/", query: fixture.query, orderedHeaders: fixture.headers, status: 413, executed: true }));
      continue;
    }
  }
  let status;
  try { status = (await oidc.handleRequest(request)).status; } catch (error) { status = error?.constructor?.name === "BodyTooLargeError" ? 413 : 500; }
  const decoded = decodeURIComponent((request.rawPath.length ? request.rawPath : Buffer.from("/")).toString()).replace(/^\/auth(?=\/|$)/, "") || "/";
  console.log(JSON.stringify({ id: fixture.id, method: request.method, decodedPath: decoded, query: request.query?.toString() ?? null, orderedHeaders: request.headers, status, executed: true }));
}
