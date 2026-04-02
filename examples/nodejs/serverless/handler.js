"use strict";

const path = require("path");
const { createHandler } = require("@oidc-exchange/lambda");

module.exports.oidcExchange = createHandler({
  config: path.resolve(__dirname, "..", "config.toml"),
  basePath: "/auth",
});
