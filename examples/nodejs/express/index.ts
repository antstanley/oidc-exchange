import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import express from "express";
import { collectBoundedExpressBody } from "./body-limit.js";
import { OidcExchange } from "@oidc-exchange/node";

const __dirname = dirname(fileURLToPath(import.meta.url));

const oidc = new OidcExchange(process.env.OIDC_EXCHANGE_CONFIG_STRING
  ? { configString: process.env.OIDC_EXCHANGE_CONFIG_STRING }
  : { config: resolve(__dirname, "..", "config.toml") });

const app = express();

app.all("/auth/*", (req, res) => {
  collectBoundedExpressBody(req, res, oidc.limits().maxBodyBytes, async (body) => {
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

if (process.env.NODE_ENV !== "test") {
  const port = process.env.PORT || 8080;
  app.listen(port, () => {
    console.log(`OIDC-Exchange (Express) listening on http://localhost:${port}`);
  });
}
