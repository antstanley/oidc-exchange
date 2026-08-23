import path from 'path'
import { Hono } from 'hono'
import { serve } from '@hono/node-server'
import { OidcExchange } from '@oidc-exchange/node'

const oidc = new OidcExchange({
  config: path.resolve(__dirname, '..', 'config.toml'),
})

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

  const body = req.body ? Buffer.from(await req.arrayBuffer()) : undefined

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

  return new Response(response.body, {
    status: response.status,
    headers: responseHeaders,
  })
})

const port = Number(process.env.PORT) || 8080

serve({ fetch: app.fetch, port }, () => {
  console.log(`OIDC-Exchange (Hono) listening on http://localhost:${port}`)
})
