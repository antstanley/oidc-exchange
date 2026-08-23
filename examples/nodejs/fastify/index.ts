import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import Fastify from "fastify";
import { OidcExchange } from "@oidc-exchange/node";

const __dirname = dirname(fileURLToPath(import.meta.url));

const oidc = new OidcExchange({
  config: resolve(__dirname, "..", "config.toml"),
});

const app = Fastify({ bodyLimit: oidc.limits().maxBodyBytes });

app.addContentTypeParser("*", { parseAs: "buffer" }, (_req, body, done) => {
  done(null, body);
});

app.all("/auth/*", async (request, reply) => {
  const queryIndex = request.raw.url?.indexOf("?") ?? -1;
  const rawTarget = request.raw.url ?? request.url;
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex);
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1);

  const headers = [];
  for (const [name, value] of Object.entries(request.headers)) {
    if (Array.isArray(value)) {
      for (const v of value) {
        headers.push({ name, value: v });
      }
    } else if (value !== undefined) {
      headers.push({ name, value });
    }
  }

  const body =
    request.body instanceof Buffer && request.body.length > 0
      ? request.body
      : undefined;

  const response = await oidc.handleRequest({
    method: request.method,
    rawPath: Buffer.from(rawPath),
    query: query === undefined ? undefined : Buffer.from(query),
    headers,
    body,
    pathIsRaw: true,
  });

  for (const { name, value } of response.headers) {
    reply.header(name, value);
  }

  reply.status(response.status).send(response.body);
});

const port = Number(process.env.PORT) || 8080;

app.listen({ host: "0.0.0.0", port }, (err) => {
  if (err) {
    console.error(err);
    process.exit(1);
  }
  console.log(`OIDC-Exchange (Fastify) listening on http://localhost:${port}`);
});
