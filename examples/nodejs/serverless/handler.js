'use strict'

const path = require('path')
const { OidcExchange } = require('@oidc-exchange/node')

const oidc = new OidcExchange({
  config: path.resolve(__dirname, '..', 'config.toml'),
})

module.exports.oidcExchange = async (event) => {
  const oidcPath = (event.pathParameters && event.pathParameters.proxy)
    ? '/' + event.pathParameters.proxy
    : '/'

  const headers = []
  if (event.headers) {
    for (const [name, value] of Object.entries(event.headers)) {
      if (value !== undefined) {
        headers.push({ name, value })
      }
    }
  }

  const body = event.body
    ? Buffer.from(event.body, event.isBase64Encoded ? 'base64' : 'utf-8')
    : undefined

  const response = oidc.handleRequest({
    method: event.httpMethod,
    path: oidcPath,
    headers,
    body,
  })

  const responseHeaders = {}
  for (const { name, value } of response.headers) {
    responseHeaders[name] = value
  }

  return {
    statusCode: response.status,
    headers: responseHeaders,
    body: response.body.toString('base64'),
    isBase64Encoded: true,
  }
}
