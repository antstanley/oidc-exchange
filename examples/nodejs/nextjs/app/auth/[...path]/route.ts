import path from 'path'
import { OidcExchange } from '@oidc-exchange/node'

const oidc = new OidcExchange({
  config: path.resolve(process.cwd(), '..', 'config.toml'),
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

async function handler(request: Request) {
  const targetStart = request.url.indexOf('/', request.url.indexOf('://') + 3)
  const rawTarget = targetStart < 0 ? '/' : request.url.slice(targetStart)
  const queryIndex = rawTarget.indexOf('?')
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex)
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1)

  const headers: { name: string; value: string }[] = []
  request.headers.forEach((value, name) => {
    headers.push({ name, value })
  })

  const body = request.body
    ? await readBounded(request.body, oidc.limits().maxBodyBytes)
    : undefined

  const response = await oidc.handleRequest({
    method: request.method,
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
}

export const GET = handler
export const POST = handler
export const PUT = handler
export const DELETE = handler
export const PATCH = handler
