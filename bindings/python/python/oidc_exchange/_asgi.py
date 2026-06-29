"""ASGI adapter for oidc-exchange."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, MutableMapping
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange

# Minimal ASGI type aliases (scope/message are open string-keyed mappings per the
# ASGI spec); kept local so the binding has no asgiref dependency.
Scope = MutableMapping[str, Any]
Message = MutableMapping[str, Any]
Receive = Callable[[], Awaitable[Message]]
Send = Callable[[Message], Awaitable[None]]
ASGIApp = Callable[[Scope, Receive, Send], Awaitable[None]]


def make_asgi_app(oidc: OidcExchange) -> ASGIApp:
    """Create an ASGI application from an OidcExchange instance."""

    async def app(scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            return

        body = b""
        while True:
            message = await receive()
            body += message.get("body", b"")
            if not message.get("more_body", False):
                break

        headers: dict[str, str] = {}
        for name, value in scope.get("headers", []):
            headers[name.decode("latin-1")] = value.decode("latin-1")

        path = scope.get("path", "/")
        query = scope.get("query_string", b"")
        if query:
            path = f"{path}?{query.decode('latin-1')}"

        request: dict[str, Any] = {
            "method": scope["method"],
            "path": path,
            "headers": headers,
            "body": body,
        }

        response = await oidc.handle_request(request)

        resp_headers = [
            (k.encode("latin-1"), v.encode("latin-1")) for k, v in response["headers"].items()
        ]

        await send(
            {
                "type": "http.response.start",
                "status": response["status"],
                "headers": resp_headers,
            }
        )
        await send(
            {
                "type": "http.response.body",
                "body": response["body"],
            }
        )

    return app
