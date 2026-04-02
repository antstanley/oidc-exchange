/* eslint-disable no-console */
'use strict'

const { platform, arch } = process

const PLATFORM_MAP = {
  'linux-x64': '@oidc-exchange/linux-x64-gnu',
  'linux-arm64': '@oidc-exchange/linux-arm64-gnu',
  'win32-x64': '@oidc-exchange/win32-x64-msvc',
  'darwin-arm64': '@oidc-exchange/darwin-arm64',
}

let nativeBinding = null
let loadError = null

const platformKey = `${platform}-${arch}`
const packageName = PLATFORM_MAP[platformKey]

if (packageName) {
  try {
    nativeBinding = require(packageName)
  } catch (_e) {
    // Fall back to locally built .node file (development builds).
    try {
      nativeBinding = require('./oidc-exchange.node')
    } catch (e) {
      loadError = e
    }
  }
} else {
  loadError = new Error(`Unsupported platform: ${platformKey}`)
}

if (!nativeBinding) {
  if (loadError) {
    throw loadError
  }
  throw new Error(`Failed to load native binding for platform: ${platformKey}`)
}

const { OidcExchange } = nativeBinding

module.exports.OidcExchange = OidcExchange
