import path from 'path'
import type { Handle } from '@sveltejs/kit'
import { OidcExchange } from '@oidc-exchange/node'

const oidc = new OidcExchange({
  config: path.resolve(process.cwd(), '..', 'config.toml'),
})

export const handle: Handle = async ({ event, resolve }) => {
  if (!event.url.pathname.startsWith('/auth/')) {
    return resolve(event)
  }

  const request = event.request
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
    ? Buffer.from(await request.arrayBuffer())
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
