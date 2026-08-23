"""WSGI adapter for oidc-exchange."""

from __future__ import annotations

from collections.abc import Callable, Iterable
from typing import TYPE_CHECKING, Any, BinaryIO

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange

MAX_REQUEST_BODY_BYTES = 2 * 1024 * 1024
READ_CHUNK_BYTES = 64 * 1024
WSGIEnvironment = dict[str, Any]
StartResponse = Callable[[str, list[tuple[str, str]]], object]
WSGIApp = Callable[[WSGIEnvironment, StartResponse], Iterable[bytes]]


def _error(start_response: StartResponse, status: str) -> list[bytes]:
    start_response(status, [])
    return [b""]


def _content_length(environ: WSGIEnvironment) -> int | None:
    raw = environ.get("CONTENT_LENGTH")
    if raw in (None, ""):
        return 0
    try:
        value = int(raw)
    except (TypeError, ValueError, OverflowError):
        return None
    return value if value >= 0 else None


def _read_body(stream: BinaryIO, length: int) -> bytes:
    body = bytearray()
    remaining = length
    while remaining:
        chunk = stream.read(min(remaining, READ_CHUNK_BYTES))
        if not chunk:
            break
        body.extend(chunk)
        remaining -= len(chunk)
    return bytes(body)


def make_wsgi_app(
    oidc: OidcExchange, max_request_body_bytes: int = MAX_REQUEST_BODY_BYTES
) -> WSGIApp:
    """Create a WSGI application with defensive content-length handling."""

    def app(environ: WSGIEnvironment, start_response: StartResponse) -> Iterable[bytes]:
        content_length = _content_length(environ)
        if content_length is None:
            return _error(start_response, "400 Bad Request")
        if content_length > max_request_body_bytes:
            return _error(start_response, "413 Payload Too Large")
        body = _read_body(environ["wsgi.input"], content_length)
        headers = {
            key[5:].replace("_", "-").lower(): value
            for key, value in environ.items()
            if key.startswith("HTTP_")
        }
        if "CONTENT_TYPE" in environ:
            headers["content-type"] = environ["CONTENT_TYPE"]
        if "CONTENT_LENGTH" in environ:
            headers["content-length"] = environ["CONTENT_LENGTH"]
        path = environ.get("PATH_INFO", "/")
        query = environ.get("QUERY_STRING", "")
        if query:
            path = f"{path}?{query}"
        response = oidc.handle_request_sync(
            {"method": environ["REQUEST_METHOD"], "path": path, "headers": headers, "body": body}
        )
        reason_phrases = {
            200: "OK",
            201: "Created",
            204: "No Content",
            301: "Moved Permanently",
            302: "Found",
            304: "Not Modified",
            400: "Bad Request",
            401: "Unauthorized",
            403: "Forbidden",
            404: "Not Found",
            405: "Method Not Allowed",
            409: "Conflict",
            413: "Payload Too Large",
            500: "Internal Server Error",
            502: "Bad Gateway",
            503: "Service Unavailable",
        }
        status_code = response["status"]
        start_response(
            f"{status_code} {reason_phrases.get(status_code, '')}",
            list(response["headers"].items()),
        )
        return [response["body"]]

    return app
