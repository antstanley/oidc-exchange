import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import express from "express";
import { OidcExchange } from "@oidc-exchange/node";

const __dirname = dirname(fileURLToPath(import.meta.url));

const oidc = new OidcExchange({
  config: resolve(__dirname, "..", "config.toml"),
});

const app = express();

app.all("/auth/*", (req, res) => {
  const chunks: Buffer[] = [];
  const limit = oidc.limits().maxBodyBytes;
  let received = 0;
  req.on("data", (chunk: Buffer) => {
    received += chunk.length;
    if (received > limit) {
      req.destroy();
      if (!res.headersSent) res.status(413).end();
      return;
    }
    chunks.push(chunk);
  });
  req.on("end", async () => {
    if (received > limit) return;
    const body = chunks.length > 0 ? Buffer.concat(chunks, received) : undefined;
    const headers = [];
    const raw = req.rawHeaders;
    for (let i = 0; i < raw.length; i += 2) {
      headers.push({ name: raw[i], value: raw[i + 1] });
    }
    const queryIndex = req.originalUrl.indexOf("?");
    const rawPath = req.originalUrl.slice(0, queryIndex < 0 ? undefined : queryIndex);
    const query = queryIndex < 0 ? undefined : req.originalUrl.slice(queryIndex + 1);
    const response = await oidc.handleRequest({
      method: req.method,
      rawPath: Buffer.from(rawPath),
      query: query === undefined ? undefined : Buffer.from(query),
      headers,
      body,
      pathIsRaw: true,
    });
    for (const { name, value } of response.headers) {
      res.setHeader(name, value);
    }
    res.status(response.status).end(response.body);
  });
});

const port = process.env.PORT || 8080;
app.listen(port, () => {
  console.log(`OIDC-Exchange (Express) listening on http://localhost:${port}`);
});
