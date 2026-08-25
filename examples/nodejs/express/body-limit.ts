import { type Request, type Response } from "express";
import { contentLengthExceedsLimit } from "../body-limit.js";

export function collectBoundedExpressBody(
  req: Request,
  res: Response,
  limit: number,
  onBody: (body: Buffer | undefined) => void,
): void {
  let chunks: Buffer[] = [];
  let received = 0;
  let settled = false;

  const cleanup = () => {
    req.off("data", onData);
    req.off("end", onEnd);
    req.off("error", onError);
    req.off("aborted", onAborted);
  };
  const reject = () => {
    if (settled) return;
    settled = true;
    cleanup();
    chunks = [];
    if (!res.headersSent) res.status(413).end();
    req.resume();
  };
  const onData = (chunk: Buffer) => {
    if (chunk.length > limit - received) {
      reject();
      return;
    }
    received += chunk.length;
    chunks.push(chunk);
  };
  const onEnd = () => {
    if (settled) return;
    settled = true;
    cleanup();
    onBody(chunks.length === 0 ? undefined : Buffer.concat(chunks, received));
  };
  const onError = () => {
    settled = true;
    cleanup();
  };
  const onAborted = () => {
    settled = true;
    cleanup();
  };

  if (contentLengthExceedsLimit(req.get("content-length"), limit)) {
    reject();
    return;
  }
  req.on("data", onData);
  req.once("end", onEnd);
  req.once("error", onError);
  req.once("aborted", onAborted);
}
