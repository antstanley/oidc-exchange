import { Buffer } from "node:buffer";

export class BodyTooLargeError extends Error {
  constructor() {
    super("request body exceeds configured limit");
    this.name = "BodyTooLargeError";
  }
}

export function contentLengthExceedsLimit(
  contentLength: string | null | undefined,
  limit: number,
): boolean {
  if (contentLength == null || !/^\d+$/.test(contentLength)) return false;
  const length = Number(contentLength);
  return Number.isSafeInteger(length) && length > limit;
}

export async function readBoundedWebBody(
  stream: ReadableStream<Uint8Array>,
  limit: number,
): Promise<Buffer> {
  const reader = stream.getReader();
  const chunks: Buffer[] = [];
  let length = 0;
  try {
    while (length <= limit) {
      const { done, value } = await reader.read();
      if (done) return Buffer.concat(chunks, length);
      if (value.byteLength > limit - length) {
        await reader.cancel("request body exceeds configured limit");
        throw new BodyTooLargeError();
      }
      chunks.push(Buffer.from(value.buffer, value.byteOffset, value.byteLength));
      length += value.byteLength;
    }
    throw new BodyTooLargeError();
  } finally {
    reader.releaseLock();
  }
}

export async function readBoundedRequestBody(
  request: Request,
  limit: number,
): Promise<Buffer | undefined> {
  if (contentLengthExceedsLimit(request.headers.get("content-length"), limit)) {
    request.body?.cancel("request content-length exceeds configured limit").catch(() => {});
    throw new BodyTooLargeError();
  }
  return request.body ? readBoundedWebBody(request.body, limit) : undefined;
}

export function payloadTooLargeResponse(): Response {
  return new Response(null, { status: 413 });
}
