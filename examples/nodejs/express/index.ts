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
  req.on("data", (chunk: Buffer) => chunks.push(chunk));
  req.on("end", () => {
    const body = chunks.length > 0 ? Buffer.concat(chunks) : undefined;
    const headers = [];
    const raw = req.rawHeaders;
    for (let i = 0; i < raw.length; i += 2) {
      headers.push({ name: raw[i], value: raw[i + 1] });
    }
    const oidcPath = req.originalUrl.replace(/^\/auth/, "") || "/";
    const response = oidc.handleRequest({ method: req.method, path: oidcPath, headers, body });
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
