"""ASGI adapter for oidc-exchange."""

from __future__ import annotations
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange


def make_asgi_app(oidc: OidcExchange):
    """Create an ASGI application from an OidcExchange instance."""

    async def app(scope, receive, send):
        if scope["type"] != "http":
            return

        body = b""
        while True:
            message = await receive()
            body += message.get("body", b"")
            if not message.get("more_body", False):
                break

        headers = {}
        for name, value in scope.get("headers", []):
            headers[name.decode("latin-1")] = value.decode("latin-1")

        path = scope.get("path", "/")
        query = scope.get("query_string", b"")
        if query:
            path = f"{path}?{query.decode('latin-1')}"

        request = {
            "method": scope["method"],
            "path": path,
            "headers": headers,
            "body": body,
        }

        response = await oidc.handle_request(request)

        resp_headers = [
            (k.encode("latin-1"), v.encode("latin-1"))
            for k, v in response["headers"].items()
        ]

        await send({
            "type": "http.response.start",
            "status": response["status"],
            "headers": resp_headers,
        })
        await send({
            "type": "http.response.body",
            "body": response["body"],
        })

    return app
