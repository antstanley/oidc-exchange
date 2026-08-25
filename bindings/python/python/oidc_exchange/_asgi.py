"""ASGI adapter preserving host-supplied wire data."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, MutableMapping
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange

Scope = MutableMapping[str, Any]
Message = MutableMapping[str, Any]
Receive = Callable[[], Awaitable[Message]]
Send = Callable[[Message], Awaitable[None]]
ASGIApp = Callable[[Scope, Receive, Send], Awaitable[None]]


async def _send_error(send: Send, status: int) -> None:
    await send({"type": "http.response.start", "status": status, "headers": []})
    await send({"type": "http.response.body", "body": b""})


async def _bounded_body(receive: Receive, limit: int) -> bytearray | None:
    body = bytearray()
    while True:
        message = await receive()
        chunk = message.get("body", b"")
        if not isinstance(chunk, bytes) or len(chunk) > limit - len(body):
            return None
        body.extend(chunk)
        if not message.get("more_body", False):
            return body


def make_asgi_app(oidc: OidcExchange, max_request_body_bytes: int | None = None) -> ASGIApp:
    """Raw fidelity requires an ASGI server that supplies scope['raw_path']."""
    limit = max_request_body_bytes or oidc.limits()["max_body_bytes"]

    async def app(scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            return
        body = await _bounded_body(receive, limit)
        if body is None:
            await _send_error(send, 413)
            return
        raw_path = scope.get("raw_path")
        path_is_raw = isinstance(raw_path, bytes)
        if not path_is_raw:
            raw_path = str(scope.get("path", "/")).encode("utf-8")
        response = await oidc.handle_request(
            {
                "method": scope["method"],
                "raw_path": raw_path,
                "query": scope.get("query_string", b""),
                "headers": [
                    (name.decode("latin-1"), value.decode("latin-1"))
                    for name, value in scope.get("headers", [])
                ],
                "body": bytes(body),
                "path_is_raw": path_is_raw,
            }
        )
        await send(
            {
                "type": "http.response.start",
                "status": response["status"],
                "headers": [
                    (name.encode("latin-1"), value.encode("latin-1"))
                    for name, value in response["headers"]
                ],
            }
        )
        await send({"type": "http.response.body", "body": response["body"]})

    return app
