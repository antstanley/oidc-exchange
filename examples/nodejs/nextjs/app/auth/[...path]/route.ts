import path from 'path'
import { OidcExchange } from '@oidc-exchange/node'

const oidc = new OidcExchange({
  config: path.resolve(process.cwd(), '..', 'config.toml'),
})

async function handler(request: Request) {
  const url = new URL(request.url)
  const oidcPath = url.pathname.replace(/^\/auth/, '') || '/'

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

export const GET = handler
export const POST = handler
export const PUT = handler
export const DELETE = handler
export const PATCH = handler
