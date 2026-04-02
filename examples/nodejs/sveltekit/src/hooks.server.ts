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
  const oidcPath = event.url.pathname.replace(/^\/auth/, '') || '/'

  const headers: { name: string; value: string }[] = []
  request.headers.forEach((value, name) => {
    headers.push({ name, value })
  })

  const body = request.body
    ? Buffer.from(await request.arrayBuffer())
    : undefined

  const response = oidc.handleRequest({
    method: request.method,
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
}
