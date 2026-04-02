import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createHandler } from "@oidc-exchange/lambda";

const __dirname = dirname(fileURLToPath(import.meta.url));

export const oidcExchange = createHandler({
  config: resolve(__dirname, "..", "config.toml"),
  basePath: "/auth",
});
