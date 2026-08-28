---
title: Python
description: Use oidc-exchange as an embedded OIDC provider in Python applications.
---

## Installation

```bash
uv add oidc-exchange
```

Or with pip:

```bash
pip install oidc-exchange
```

Requires **Python 3.10+**. Prebuilt wheels are included for Linux (x64, ARM64), macOS (ARM64), and Windows (x64).

## Basic Usage

```python
from oidc_exchange import OidcExchange

oidc = OidcExchange(config="./config.toml")

response = oidc.handle_request_sync({
    "method": "GET",
    "raw_path": b"/health",
    "query": b"",
    "headers": [],
    "body": b"",
    "path_is_raw": True,
})
print(response["status"])  # 200
```

The `handle_request_sync` method takes a dict with `method`, `raw_path` (bytes, the still-percent-encoded path when `path_is_raw` is `True`), `query` (bytes without the leading `?`), `headers` (a list of `(name, value)` tuples), `body` (bytes), and `path_is_raw` (bool). It returns a dict with `status` (int), `headers` (a list of `(name, value)` pairs), and `body` (bytes).

## Framework Integration

### FastAPI

```python
from fastapi import FastAPI
from oidc_exchange import OidcExchange

app = FastAPI()
oidc = OidcExchange(config="../config.toml")
app.mount("/auth", oidc.asgi_app())
```

The `asgi_app()` method returns a standard ASGI application that can be mounted directly into any ASGI framework.

### Flask

```python
from flask import Flask
from werkzeug.middleware.dispatcher import DispatcherMiddleware
from oidc_exchange import OidcExchange

app = Flask(__name__)
oidc = OidcExchange(config="../config.toml")
app.wsgi_app = DispatcherMiddleware(app.wsgi_app, {"/auth": oidc.wsgi_app()})
```

### Django

Add a catch-all view in your `urls.py`:

```python
import os
from django.http import HttpResponse
from django.urls import re_path
from oidc_exchange import OidcExchange

oidc = OidcExchange(config=os.path.join(os.path.dirname(__file__), "..", "..", "config.toml"))

def oidc_view(request, oidc_path=""):
    headers = []
    for key, value in request.META.items():
        if key.startswith("HTTP_"):
            header_name = key[5:].replace("_", "-").lower()
            headers.append((header_name, value))
    if "CONTENT_TYPE" in request.META:
        headers.append(("content-type", request.META["CONTENT_TYPE"]))
    if "CONTENT_LENGTH" in request.META:
        headers.append(("content-length", request.META["CONTENT_LENGTH"]))

    # Django hands you an already-decoded path segment, so mark path_is_raw False
    # and pass the raw query string separately.
    response = oidc.handle_request_sync({
        "method": request.method,
        "raw_path": f"/{oidc_path}".encode("utf-8"),
        "query": request.META.get("QUERY_STRING", "").encode("latin-1"),
        "headers": headers,
        "body": request.body,
        "path_is_raw": False,
    })

    django_response = HttpResponse(content=response["body"], status=response["status"])
    for name, value in response["headers"]:
        django_response[name] = value
    return django_response

urlpatterns = [
    re_path(r"^auth/(?P<oidc_path>.*)$", oidc_view),
]
```

## Async Support

The `handle_request` method is async and runs the handler in a thread pool executor:

```python
response = await oidc.handle_request({
    "method": "GET",
    "raw_path": b"/health",
    "query": b"",
    "headers": [],
    "body": b"",
    "path_is_raw": True,
})
```

Use `handle_request_sync` for synchronous contexts (Flask, Django). The ASGI adapter uses `handle_request` (async) internally, so FastAPI gets non-blocking behavior automatically.

## Configuration

```python
# File path
oidc = OidcExchange(config="./config.toml")

# Inline TOML
oidc = OidcExchange(config_string="""
[server]
issuer = "https://auth.example.com"
role = "exchange"
[repository]
adapter = "sqlite"
[repository.sqlite]
path = ":memory:"
""")
```

See the [Configuration guide](/guides/configuration) for all available options.
