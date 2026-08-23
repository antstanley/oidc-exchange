import asyncio
import importlib.util
import io
import json
import os
import sys
import time
from typing import Any
from urllib.parse import unquote

shape, config, artifact, variant = sys.argv[1:]
if not os.path.isfile(artifact) or time.time() - os.path.getmtime(artifact) > 900:
    raise RuntimeError(f"fresh PyO3 artifact missing or stale: {artifact}")
spec = importlib.util.spec_from_file_location("oidc_exchange._oidc_exchange", artifact)
if spec is None or spec.loader is None:
    raise RuntimeError("cannot load PyO3 artifact")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
sys.modules["oidc_exchange._oidc_exchange"] = module
sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "bindings", "python", "python"),
)

from oidc_exchange import OidcExchange  # noqa: E402
from oidc_exchange._asgi import make_asgi_app  # noqa: E402
from oidc_exchange._wsgi import make_wsgi_app  # noqa: E402


def body(fixture: dict[str, Any]) -> bytes:
    return b"x" * fixture["bodyLength"]


def decoded(value: str) -> str:
    return unquote(value or "/")


def parsed(fixture_id: str, status: int, data: bytes) -> dict[str, Any]:
    if data:
        try:
            return {"id": fixture_id, **json.loads(data), "executed": True}
        except json.JSONDecodeError:
            pass
    return {"id": fixture_id, "status": status, "executed": True}


async def asgi(app: Any, fixture: dict[str, Any]) -> dict[str, Any]:
    request_body = body(fixture)
    split = min(len(request_body), 64 * 1024)
    messages = [
        {
            "type": "http.request",
            "body": request_body[:split],
            "more_body": split < len(request_body),
        },
    ]
    if split < len(request_body):
        messages.append({"type": "http.request", "body": request_body[split:], "more_body": False})
    sent: list[dict[str, Any]] = []

    async def receive() -> dict[str, Any]:
        if messages:
            return messages.pop(0)
        return {"type": "http.disconnect"}

    async def send(message: dict[str, Any]) -> None:
        sent.append(message)

    scope = {
        "type": "http",
        "method": fixture["method"],
        "path": decoded(fixture["rawPath"]),
        "query_string": (fixture.get("query") or "").encode("latin-1"),
        "headers": [
            (header["name"].encode("latin-1"), header["value"].encode("latin-1"))
            for header in fixture["headers"]
        ]
        + [(b"x-oidc-conformance-observe", b"1")],
    }
    if variant == "faithful":
        scope["raw_path"] = fixture["rawPath"].encode("latin-1")
    await app(scope, receive, send)
    status = next(message["status"] for message in sent if message["type"] == "http.response.start")
    data = b"".join(
        message.get("body", b"") for message in sent if message["type"] == "http.response.body"
    )
    return parsed(fixture["id"], status, data)


def wsgi(app: Any, fixture: dict[str, Any]) -> dict[str, Any]:
    captured: dict[str, Any] = {}

    def start_response(status: str, headers: list[tuple[str, str]]) -> None:
        captured.update(status=int(status.split()[0]), headers=headers)

    content_length = next(
        (
            header["value"]
            for header in fixture["headers"]
            if header["name"].lower() == "content-length"
        ),
        str(fixture["bodyLength"]),
    )
    environ = {
        "HTTP_X_OIDC_CONFORMANCE_OBSERVE": "1",
        "REQUEST_METHOD": fixture["method"],
        "PATH_INFO": decoded(fixture["rawPath"]),
        "QUERY_STRING": fixture.get("query") or "",
        "wsgi.input": io.BytesIO(body(fixture)),
        "CONTENT_LENGTH": content_length,
    }
    if variant == "faithful":
        query = f"?{fixture['query']}" if fixture.get("query") else ""
        environ["RAW_URI"] = fixture["rawPath"] + query
        environ["oidc_exchange.headers"] = [
            (header["name"], header["value"]) for header in fixture["headers"]
        ] + [("x-oidc-conformance-observe", "1")]
    else:
        for header in fixture["headers"]:
            if header["name"].lower() not in ("content-length", "content-type"):
                key = "HTTP_" + header["name"].upper().replace("-", "_")
                environ[key] = header["value"]
    data = b"".join(app(environ, start_response))
    return parsed(fixture["id"], captured["status"], data)


async def main() -> None:
    oidc = OidcExchange(config=config)
    app = make_asgi_app(oidc) if shape == "asgi" else make_wsgi_app(oidc)
    for line in sys.stdin:
        fixture = json.loads(line)
        output = await asgi(app, fixture) if shape == "asgi" else wsgi(app, fixture)
        print(json.dumps(output), flush=True)


asyncio.run(main())
