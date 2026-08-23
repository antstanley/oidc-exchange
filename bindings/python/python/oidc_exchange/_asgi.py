"""ASGI adapter for oidc-exchange."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, MutableMapping
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange

MAX_REQUEST_BODY_BYTES = 2 * 1024 * 1024
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


def make_asgi_app(
    oidc: OidcExchange, max_request_body_bytes: int = MAX_REQUEST_BODY_BYTES
) -> ASGIApp:
    """Create an ASGI application with bounded request buffering."""

    async def app(scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            return
        body = await _bounded_body(receive, max_request_body_bytes)
        if body is None:
            await _send_error(send, 413)
            return
        headers = {
            name.decode("latin-1"): value.decode("latin-1")
            for name, value in scope.get("headers", [])
        }
        path = scope.get("path", "/")
        query = scope.get("query_string", b"")
        if query:
            path = f"{path}?{query.decode('latin-1')}"
        response = await oidc.handle_request(
            {"method": scope["method"], "path": path, "headers": headers, "body": bytes(body)}
        )
        resp_headers = [
            (key.encode("latin-1"), value.encode("latin-1"))
            for key, value in response["headers"].items()
        ]
        await send(
            {"type": "http.response.start", "status": response["status"], "headers": resp_headers}
        )
        await send({"type": "http.response.body", "body": response["body"]})

    return app
