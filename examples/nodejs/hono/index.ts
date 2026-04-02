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

  const url = new URL(req.url)
  const oidcPath = url.pathname.replace(/^\/auth/, '') || '/'

  const headers: { name: string; value: string }[] = []
  req.headers.forEach((value, name) => {
    headers.push({ name, value })
  })

  const body = req.body ? Buffer.from(await req.arrayBuffer()) : undefined

  const response = oidc.handleRequest({
    method: req.method,
    path: oidcPath,
    headers,
    body,
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
