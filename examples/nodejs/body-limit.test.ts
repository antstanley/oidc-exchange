import assert from "node:assert/strict";
import test from "node:test";
import { BodyTooLargeError, contentLengthExceedsLimit, payloadTooLargeResponse, readBoundedRequestBody, readBoundedWebBody } from "./body-limit.js";

function stream(chunks: number[], observation: { cancelled?: boolean } = {}) {
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      const size = chunks.shift();
      if (size === undefined) controller.close();
      else controller.enqueue(new Uint8Array(size).fill(7));
    },
    cancel() { observation.cancelled = true; },
  });
}

test("web reader accepts below, exact, empty, and many chunks", async () => {
  assert.equal((await readBoundedWebBody(stream([2, 3]), 6)).length, 5);
  assert.equal((await readBoundedWebBody(stream([1, 2, 3]), 6)).length, 6);
  assert.equal((await readBoundedWebBody(stream([]), 6)).length, 0);
  assert.equal((await readBoundedWebBody(stream(Array(20).fill(1)), 20)).length, 20);
});

test("web reader cancels and releases on one byte or many chunks above cap", async () => {
  for (const chunks of [[7], [2, 2, 2, 1]]) {
    const observation = {};
    const body = stream(chunks, observation);
    await assert.rejects(readBoundedWebBody(body, 6), BodyTooLargeError);
    assert.equal(observation.cancelled, true);
    const reader = body.getReader();
    reader.releaseLock();
  }
});

test("request helper handles no body and absent content-length", async () => {
  assert.equal(await readBoundedRequestBody(new Request("https://example.test", { method: "GET" }), 6), undefined);
  const request = new Request("https://example.test", { method: "POST", body: stream([3, 3]), duplex: "half" } as RequestInit);
  assert.equal((await readBoundedRequestBody(request, 6))?.length, 6);
});

test("truthful oversized content-length fast rejects without reading", async () => {
  let pulls = 0;
  const body = new ReadableStream<Uint8Array>({ pull() { pulls += 1; }, cancel() {} });
  const request = new Request("https://example.test", { method: "POST", headers: { "content-length": "7" }, body, duplex: "half" } as RequestInit);
  await assert.rejects(readBoundedRequestBody(request, 6), BodyTooLargeError);
  assert.equal(pulls, 0);
});

test("lying small content-length remains stream bounded", async () => {
  const request = new Request("https://example.test", { method: "POST", headers: { "content-length": "1" }, body: stream([7]), duplex: "half" } as RequestInit);
  await assert.rejects(readBoundedRequestBody(request, 6), BodyTooLargeError);
});

test("content-length parser and exact 413 mapping", () => {
  assert.equal(contentLengthExceedsLimit(undefined, 6), false);
  assert.equal(contentLengthExceedsLimit("6", 6), false);
  assert.equal(contentLengthExceedsLimit("7", 6), true);
  assert.equal(contentLengthExceedsLimit("invalid", 6), false);
  const response = payloadTooLargeResponse();
  assert.equal(response.status, 413);
  assert.equal(response.body, null);
});
