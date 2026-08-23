import path from 'path'
import { Hono } from 'hono'
import { serve } from '@hono/node-server'
import { OidcExchange } from '@oidc-exchange/node'

const oidc = new OidcExchange({
  config: path.resolve(__dirname, '..', 'config.toml'),
})

async function readBounded(stream: ReadableStream<Uint8Array>, limit: number): Promise<Buffer> {
  const reader = stream.getReader()
  const chunks: Buffer[] = []
  let length = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) return Buffer.concat(chunks, length)
      length += value.byteLength
      if (length > limit) {
        await reader.cancel('request body exceeds configured limit')
        throw new Response(null, { status: 413 })
      }
      chunks.push(Buffer.from(value))
    }
  } finally {
    reader.releaseLock()
  }
}

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

  const body = req.body ? await readBounded(req.body, oidc.limits().maxBodyBytes) : undefined

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
