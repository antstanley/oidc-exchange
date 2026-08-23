import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import { collectBoundedExpressBody } from "./body-limit.js";

class RequestStub extends EventEmitter {
  headers: Record<string, string> = {};
  resumed = false;
  get(name: string) { return this.headers[name.toLowerCase()]; }
  resume() { this.resumed = true; }
}
class ResponseStub {
  headersSent = false;
  statusCode = 0;
  ended = 0;
  status(code: number) { this.statusCode = code; return this; }
  end() { this.ended += 1; this.headersSent = true; }
}
function collect(chunks: number[], declared?: string) {
  const req = new RequestStub();
  const res = new ResponseStub();
  if (declared !== undefined) req.headers["content-length"] = declared;
  let body: Buffer | undefined;
  collectBoundedExpressBody(req as never, res as never, 6, (value) => { body = value; });
  for (const size of chunks) req.emit("data", Buffer.alloc(size));
  req.emit("end");
  return { req, res, body };
}

test("Express collector accepts below, exact, empty, and chunked bodies", () => {
  assert.equal(collect([2, 3]).body?.length, 5);
  assert.equal(collect([2, 2, 2], "6").body?.length, 6);
  assert.equal(collect([]).body, undefined);
  assert.equal(collect([1, 1, 1, 1, 1, 1]).body?.length, 6);
});

test("Express collector maps streaming overflow to one 413 and removes listeners", () => {
  for (const chunks of [[7], [2, 2, 2, 1]]) {
    const { req, res, body } = collect(chunks);
    assert.equal(body, undefined);
    assert.equal(res.statusCode, 413);
    assert.equal(res.ended, 1);
    assert.equal(req.resumed, true);
    assert.equal(req.listenerCount("data"), 0);
    assert.equal(req.listenerCount("end"), 0);
    assert.equal(req.listenerCount("error"), 0);
    assert.equal(req.listenerCount("aborted"), 0);
  }
});

test("Express collector fast rejects truthful oversized length and bounds lies", () => {
  const fast = collect([], "7");
  assert.equal(fast.res.statusCode, 413);
  assert.equal(fast.res.ended, 1);
  assert.equal(fast.req.listenerCount("data"), 0);
  const lie = collect([7], "1");
  assert.equal(lie.res.statusCode, 413);
});
