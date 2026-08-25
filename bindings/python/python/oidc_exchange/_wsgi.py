"""WSGI adapter with qualified raw-path and header fidelity."""

from __future__ import annotations

from collections.abc import Callable, Iterable
from typing import TYPE_CHECKING, Any, BinaryIO, cast

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange

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
    while length:
        chunk = stream.read(min(length, READ_CHUNK_BYTES))
        if not chunk:
            break
        body.extend(chunk)
        length -= len(chunk)
    return bytes(body)


def _wire_path(environ: WSGIEnvironment) -> tuple[bytes, bool]:
    raw_uri = environ.get("RAW_URI") or environ.get("REQUEST_URI")
    if isinstance(raw_uri, str):
        return raw_uri.partition("?")[0].encode("latin-1"), True
    return str(environ.get("PATH_INFO", "/")).encode("utf-8"), False


def _headers(environ: WSGIEnvironment) -> list[tuple[str, str]]:
    # Standard WSGI collapses request headers. Servers may expose an ordered
    # extension as oidc_exchange.headers; otherwise duplicate fidelity is unavailable.
    supplied = environ.get("oidc_exchange.headers")
    if isinstance(supplied, list):
        return cast(list[tuple[str, str]], supplied)
    headers = [
        (key[5:].replace("_", "-").lower(), str(value))
        for key, value in environ.items()
        if key.startswith("HTTP_")
    ]
    if "CONTENT_TYPE" in environ:
        headers.append(("content-type", str(environ["CONTENT_TYPE"])))
    if "CONTENT_LENGTH" in environ:
        headers.append(("content-length", str(environ["CONTENT_LENGTH"])))
    return headers


def make_wsgi_app(oidc: OidcExchange, max_request_body_bytes: int | None = None) -> WSGIApp:
    """RAW_URI/REQUEST_URI support is server-specific; PATH_INFO fallback is decoded."""
    limit = max_request_body_bytes or oidc.limits()["max_body_bytes"]

    def app(environ: WSGIEnvironment, start_response: StartResponse) -> Iterable[bytes]:
        content_length = _content_length(environ)
        if content_length is None:
            return _error(start_response, "400 Bad Request")
        if content_length > limit:
            return _error(start_response, "413 Payload Too Large")
        raw_path, path_is_raw = _wire_path(environ)
        response = oidc.handle_request_sync(
            {
                "method": environ["REQUEST_METHOD"],
                "raw_path": raw_path,
                "query": str(environ.get("QUERY_STRING", "")).encode("latin-1"),
                "headers": _headers(environ),
                "body": _read_body(environ["wsgi.input"], content_length),
                "path_is_raw": path_is_raw,
            }
        )
        status_code = response["status"]
        reasons = {
            200: "OK",
            400: "Bad Request",
            404: "Not Found",
            413: "Payload Too Large",
            500: "Internal Server Error",
        }
        start_response(f"{status_code} {reasons.get(status_code, '')}", list(response["headers"]))
        return [response["body"]]

    return app
