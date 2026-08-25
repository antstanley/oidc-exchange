import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Hono } from 'hono'
import { serve } from '@hono/node-server'
import { OidcExchange } from '@oidc-exchange/node'
import { BodyTooLargeError, payloadTooLargeResponse, readBoundedRequestBody } from '../body-limit.js'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

const oidc = new OidcExchange(process.env.OIDC_EXCHANGE_CONFIG_STRING
  ? { configString: process.env.OIDC_EXCHANGE_CONFIG_STRING }
  : { config: path.resolve(__dirname, '..', 'config.toml') })

const app = new Hono()

app.all('/auth/*', async (c) => {
  const req = c.req.raw

  const targetStart = req.url.indexOf('/', req.url.indexOf('://') + 3)
  const rawTarget = targetStart < 0 ? '/' : req.url.slice(targetStart)
  const queryIndex = rawTarget.indexOf('?')
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex)
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1)

  const headers: { name: string; value: string }[] = []
  req.headers.forEach((value, name) => {
    headers.push({ name, value })
  })

  let body: Buffer | undefined
  try {
    body = await readBoundedRequestBody(req, oidc.limits().maxBodyBytes)
  } catch (error) {
    if (error instanceof BodyTooLargeError) return payloadTooLargeResponse()
    throw error
  }

  const response = await oidc.handleRequest({
    method: req.method,
    rawPath: Buffer.from(rawPath),
    query: query === undefined ? undefined : Buffer.from(query),
    headers,
    body,
    pathIsRaw: true,
  })

  const responseHeaders = new Headers()
  for (const { name, value } of response.headers) {
    responseHeaders.append(name, value)
  }

  return new Response(new Uint8Array(response.body), {
    status: response.status,
    headers: responseHeaders,
  })
})

if (process.env.NODE_ENV !== 'test') {
  const port = Number(process.env.PORT) || 8080
  serve({ fetch: app.fetch, port }, () => {
    console.log(`OIDC-Exchange (Hono) listening on http://localhost:${port}`)
  })
}
