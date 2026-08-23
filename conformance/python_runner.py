from __future__ import annotations

import asyncio
import io
import json
import sys
from collections.abc import Callable
from typing import Any

from oidc_exchange import OidcExchange
from oidc_exchange._asgi import make_asgi_app
from oidc_exchange._wsgi import make_wsgi_app


def body(fixture: dict[str, Any]) -> bytes:
    return b"x" * fixture["bodyLength"]


def result(
    fixture: dict[str, Any], status: int, *, path: str, headers: list[dict[str, str]]
) -> dict[str, Any]:
    return {
        "id": fixture["id"],
        "method": fixture["method"],
        "decodedPath": path,
        "query": fixture.get("query"),
        "orderedHeaders": headers,
        "status": status,
        "executed": True,
    }


async def run_asgi(app: Callable[..., Any], fixture: dict[str, Any]) -> int:
    messages = [{"type": "http.request", "body": body(fixture), "more_body": False}]
    sent: list[dict[str, Any]] = []

    async def receive() -> dict[str, Any]:
        return messages.pop(0)

    async def send(message: dict[str, Any]) -> None:
        sent.append(message)

    scope: dict[str, Any] = {
        "type": "http",
        "method": fixture["method"],
        "path": decode_path(fixture["rawPath"]),
        "query_string": (fixture.get("query") or "").encode("latin-1"),
        "headers": [
            (h["name"].encode("latin-1"), h["value"].encode("latin-1")) for h in fixture["headers"]
        ],
    }
    if fixture["pathIsRaw"]:
        scope["raw_path"] = fixture["rawPath"].encode("latin-1")
    await app(scope, receive, send)
    return int(sent[0]["status"])


def run_wsgi(app: Callable[..., Any], fixture: dict[str, Any]) -> int:
    captured = ""

    def start_response(status: str, _headers: list[tuple[str, str]]) -> None:
        nonlocal captured
        captured = status

    environ: dict[str, Any] = {
        "REQUEST_METHOD": fixture["method"],
        "PATH_INFO": decode_path(fixture["rawPath"]),
        "QUERY_STRING": fixture.get("query", ""),
        "wsgi.input": io.BytesIO(body(fixture)),
        "CONTENT_LENGTH": content_length(fixture),
    }
    if fixture["pathIsRaw"]:
        environ["RAW_URI"] = fixture["rawPath"] + (
            ("?" + fixture["query"]) if fixture.get("query") else ""
        )
    if fixture.get("orderedHeadersAvailable", True):
        environ["oidc_exchange.headers"] = [(h["name"], h["value"]) for h in fixture["headers"]]
    list(app(environ, start_response))
    return int(captured.split()[0])


def content_length(fixture: dict[str, Any]) -> str:
    for header in fixture["headers"]:
        if header["name"].lower() == "content-length":
            return header["value"]
    return str(fixture["bodyLength"])


def decode_path(value: str) -> str:
    from urllib.parse import unquote

    return unquote(value or "/")


async def main() -> None:
    shape, config = sys.argv[1:]
    oidc = OidcExchange(config=config)
    app = make_asgi_app(oidc) if shape == "asgi" else make_wsgi_app(oidc)
    for line in sys.stdin:
        fixture = json.loads(line)
        status = await run_asgi(app, fixture) if shape == "asgi" else run_wsgi(app, fixture)
        path = decode_path(fixture["rawPath"] or "/")
        if path == "/auth":
            path = "/"
        elif path.startswith("/auth/"):
            path = path[5:]
        headers = (
            fixture["headers"]
            if shape == "asgi" or fixture.get("orderedHeadersAvailable", True)
            else []
        )
        print(json.dumps(result(fixture, status, path=path, headers=headers)), flush=True)


asyncio.run(main())
