"""WSGI adapter for oidc-exchange."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from oidc_exchange import OidcExchange


def make_wsgi_app(oidc: OidcExchange):
    """Create a WSGI application from an OidcExchange instance."""

    def app(environ, start_response):
        content_length = int(environ.get("CONTENT_LENGTH") or 0)
        body = environ["wsgi.input"].read(content_length) if content_length > 0 else b""

        headers = {}
        for key, value in environ.items():
            if key.startswith("HTTP_"):
                header_name = key[5:].replace("_", "-").lower()
                headers[header_name] = value
        if "CONTENT_TYPE" in environ:
            headers["content-type"] = environ["CONTENT_TYPE"]
        if "CONTENT_LENGTH" in environ:
            headers["content-length"] = environ["CONTENT_LENGTH"]

        path = environ.get("PATH_INFO", "/")
        query = environ.get("QUERY_STRING", "")
        if query:
            path = f"{path}?{query}"

        request = {
            "method": environ["REQUEST_METHOD"],
            "path": path,
            "headers": headers,
            "body": body,
        }

        response = oidc.handle_request_sync(request)

        status_code = response["status"]
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
            500: "Internal Server Error",
            502: "Bad Gateway",
            503: "Service Unavailable",
        }
        reason = reason_phrases.get(status_code, "")
        status_line = f"{status_code} {reason}"
        resp_headers = list(response["headers"].items())

        start_response(status_line, resp_headers)
        return [response["body"]]

    return app
