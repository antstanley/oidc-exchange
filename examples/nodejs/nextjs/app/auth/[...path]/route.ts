import path from 'path'
import { OidcExchange } from '@oidc-exchange/node'
import { BodyTooLargeError, payloadTooLargeResponse, readBoundedRequestBody } from '../../../body-limit'

const oidc = new OidcExchange(process.env.OIDC_EXCHANGE_CONFIG_STRING
  ? { configString: process.env.OIDC_EXCHANGE_CONFIG_STRING }
  : { config: path.resolve(process.cwd(), '..', 'config.toml') })

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

  let body: Buffer | undefined
  try {
    body = await readBoundedRequestBody(request, oidc.limits().maxBodyBytes)
  } catch (error) {
    if (error instanceof BodyTooLargeError) return payloadTooLargeResponse()
    throw error
  }

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

  return new Response(new Uint8Array(response.body), {
    status: response.status,
    headers: responseHeaders,
  })
}

export const GET = handler
export const POST = handler
export const PUT = handler
export const DELETE = handler
export const PATCH = handler
