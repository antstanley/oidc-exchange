import { App } from "aws-cdk-lib";
import { DownloadsStack, ACCOUNT, REGION } from "../lib/downloads-stack.ts";

const app = new App();
const hostedZoneId = String(app.node.tryGetContext("hostedZoneId") ?? "");
const zoneName = String(app.node.tryGetContext("zoneName") ?? "");
if (!hostedZoneId || !zoneName) {
  throw new Error("Pass -c hostedZoneId=<id> (and optionally -c zoneName=<zone>) to cdk");
}

new DownloadsStack(app, "OidcExchangeDownloads", {
  env: { account: ACCOUNT, region: REGION },
  hostedZoneId,
  zoneName,
});
