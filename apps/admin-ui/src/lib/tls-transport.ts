import { request as httpRequest } from "node:https";
import type { RequestOptions } from "node:https";

/**
 * The client-certificate credential the generated client resolves from the
 * server-side environment. Kept here — not in generated code — because it
 * describes Node's TLS plumbing, not the service contract.
 */
export interface ClientCertificateCredential {
  kind: "client_certificate";
  certificate: string;
  privateKey: string;
}

export type TlsRequestInit = RequestInit & {
  credential?: ClientCertificateCredential;
};

/**
 * Perform one request against the internal API presenting the configured
 * client certificate via mutual TLS.
 *
 * Why `node:https` rather than `fetch`: Node's global fetch is undici's
 * dispatcher engine, but the `undici` module itself is not importable from
 * this package, and fetch offers no standard way to present a client
 * certificate. This runs only inside the SvelteKit server runtime, where the
 * operator credential lives anyway; the browser never sees this code path or
 * any part of the credential.
 *
 * The returned Response is the platform's own, built from the collected body,
 * so the caller's response handling (status, headers, json()) is identical
 * whether the request went over TLS-with-client-cert or plain fetch.
 */
export function requestViaTls(url: string, init: TlsRequestInit = {}): Promise<Response> {
  const { credential, ...rest } = init;
  if (credential === undefined || credential.kind !== "client_certificate") {
    return Promise.reject(
      new Error("requestViaTls requires a resolved client-certificate credential"),
    );
  }
  if (rest.method === undefined) {
    rest.method = "GET";
  }

  const target = new URL(url);
  const options: RequestOptions = {
    method: rest.method,
    hostname: target.hostname,
    port: target.port === "" ? 443 : Number(target.port),
    path: `${target.pathname}${target.search}`,
    headers: normalizeHeaders(rest.headers),
    cert: credential.certificate,
    key: credential.privateKey,
    // The admin listener's identity is fixed by configuration; a deployment
    // terminating TLS elsewhere never reaches this path.
    servername: target.hostname,
  };

  return new Promise<Response>((resolve, reject) => {
    const request = httpRequest(options, (message) => {
      const chunks: Array<Uint8Array> = [];
      message.on("data", (chunk: Buffer) => {
        chunks.push(new Uint8Array(chunk));
      });
      message.on("end", () => {
        const body = Buffer.concat(chunks.map((c) => Buffer.from(c)));
        const response = new Response(bytesBody(body), {
          status: message.statusCode ?? 0,
          statusText: message.statusMessage ?? "",
          headers: flattenHeaders(message.headers),
        });
        resolve(response);
      });
    });
    request.on("error", (error) => {
      reject(error instanceof Error ? error : new Error(String(error)));
    });
    if (typeof rest.body === "string") {
      request.write(rest.body);
    }
    request.end();
  });
}

/**
 * Wrap the collected bytes as a Response body.
 *
 * Why not hand the Buffer's own view to `Response`: a Buffer's `.buffer` may
 * be any ArrayBufferLike, but Response bodies accept only ArrayBuffer-backed
 * views, so build one right-sized view of our own (one copy of an internal
 * API's JSON payload — negligible next to the TLS round trip).
 */
function bytesBody(body: Buffer): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(new ArrayBuffer(body.byteLength));
  bytes.set(body);
  return bytes;
}

function normalizeHeaders(headers: HeadersInit | undefined): Record<string, string> {
  const flat: Record<string, string> = {};
  if (headers === undefined) {
    return flat;
  }
  if (headers instanceof Headers) {
    headers.forEach((value, key) => {
      flat[key] = value;
    });
    return flat;
  }
  if (Array.isArray(headers)) {
    for (const [key, value] of headers) {
      flat[key] = value;
    }
    return flat;
  }
  return { ...headers };
}

function flattenHeaders(raw: IncomingHttpHeadersLike): Record<string, string> {
  const flat: Record<string, string> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (Array.isArray(value)) {
      flat[key] = value.join(", ");
    } else if (typeof value === "string") {
      flat[key] = value;
    } else if (value !== undefined) {
      flat[key] = String(value);
    }
  }
  return flat;
}

/** Structural subset of node:http's IncomingHttpHeaders we rely on. */
interface IncomingHttpHeadersLike extends Record<string, string | string[] | undefined> {}
