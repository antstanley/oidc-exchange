import { createInterface } from "node:readline";
import { existsSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";

const [shape, config, artifact, variant = "faithful"] = process.argv.slice(2);
if (!artifact || !existsSync(artifact)) throw new Error(`fresh napi artifact missing: ${artifact}`);
if (Date.now() - statSync(artifact).mtimeMs > 5 * 60 * 1000) throw new Error(`napi artifact is stale: ${artifact}`);
const require = createRequire(import.meta.url);
const { OidcExchange } = require(resolve(artifact));
const oidc = new OidcExchange({ config });
let createHandler;
if (shape === "lambda") ({ createHandler } = await import("../bindings/lambda/dist/index.js"));

function request(fixture) {
  return { method: fixture.method, rawPath: Buffer.from(fixture.rawPath), query: fixture.query == null ? undefined : Buffer.from(fixture.query), headers: [...fixture.headers,{name:"x-oidc-conformance-observe",value:"1"}], body: Buffer.alloc(fixture.bodyLength, 120), pathIsRaw: fixture.pathIsRaw };
}
function parse(id, response) {
  if (!response.body?.length) return { id, status: response.status, executed: true };
  const observed = JSON.parse(Buffer.from(response.body).toString());
  return { id, ...observed, executed: true };
}
function headerMaps(original) { const entries=[...original,{name:"x-oidc-conformance-observe",value:"1"}];
  const multi = {};
  for (const {name,value} of entries) (multi[name] ??= []).push(value);
  return { headers: Object.fromEntries(entries.map(({name,value}) => [name,value])), multi };
}
function queryMaps(raw) {
  if (!raw) return { single:null, multi:null };
  const multi = {};
  for (const [name,value] of new URLSearchParams(raw)) (multi[name] ??= []).push(value);
  return { single:Object.fromEntries(Object.entries(multi).map(([name,values]) => [name,values.at(-1)])), multi };
}
function decodedPath(rawPath) {
  try { return decodeURIComponent(rawPath || "/"); } catch { return rawPath || "/"; }
}
function event(f) {
  const {headers,multi}=headerMaps(f.headers); const query=queryMaps(f.query); const body=f.bodyLength?Buffer.alloc(f.bodyLength,120).toString("base64"):null;
  const hostPath=decodedPath(f.rawPath);
  if (variant === "fallback") return { httpMethod:f.method, resource:hostPath, path:hostPath, headers, multiValueHeaders:multi, queryStringParameters:query.single, multiValueQueryStringParameters:query.multi, body, isBase64Encoded:true, requestContext:{}, stageVariables:null, pathParameters:null };
  if (f.lambdaEvent === "alb") return { httpMethod:f.method, path:hostPath, headers, multiValueHeaders:multi, queryStringParameters:query.single, multiValueQueryStringParameters:query.multi, body, isBase64Encoded:true, requestContext:{elb:{targetGroupArn:"arn:test"}} };
  if (f.lambdaEvent === "v1") return { httpMethod:f.method, resource:hostPath, path:hostPath, headers, multiValueHeaders:multi, queryStringParameters:query.single, multiValueQueryStringParameters:query.multi, body, isBase64Encoded:true, requestContext:{}, stageVariables:null, pathParameters:null };
  return { version:"2.0", routeKey:"$default", rawPath:f.rawPath||"/", rawQueryString:f.query||"", headers, cookies:multi.cookie, requestContext:{http:{method:f.method,path:hostPath,protocol:"HTTP/1.1",sourceIp:"127.0.0.1",userAgent:"conformance"},accountId:"",apiId:"",domainName:"",domainPrefix:"",requestId:"",routeKey:"",stage:"",time:"",timeEpoch:0},body:body??undefined,isBase64Encoded:true };
}
for await (const line of createInterface({ input: process.stdin })) {
  const f=JSON.parse(line);
  try {
    if (shape === "node") console.log(JSON.stringify(parse(f.id, await oidc.handleRequest(request(f)))));
    else {
      const response=await createHandler({config})(event(f), {});
      const bytes=Buffer.from(response.body||"", response.isBase64Encoded?"base64":"utf8");
      const actual=parse(f.id,{status:response.statusCode,body:bytes});
      if (response.multiValueHeaders) actual.multiValueHeaders=response.multiValueHeaders;
      console.log(JSON.stringify(actual));
    }
  } catch (error) { console.log(JSON.stringify({id:f.id,status:error?.constructor?.name==="BodyTooLargeError"?413:500,executed:true})); }
}
