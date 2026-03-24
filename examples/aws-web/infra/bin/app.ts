#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib';
import { OidcExchangeExampleStack } from '../lib/stack';

const app = new cdk.App();

const googleClientId = app.node.tryGetContext('googleClientId') || 'YOUR_GOOGLE_CLIENT_ID';
const googleClientSecret = app.node.tryGetContext('googleClientSecret') || 'YOUR_GOOGLE_CLIENT_SECRET';

new OidcExchangeExampleStack(app, 'OidcExchangeExample', {
  googleClientId,
  googleClientSecret,
});
