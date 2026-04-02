---
title: Python
description: Use oidc-exchange as an embedded OIDC provider in Python applications.
---

## Installation

```bash
pip install oidc-exchange
```

Requires **Python 3.10+**.

## Basic Usage

```python
from oidc_exchange import OidcExchange

oidc = OidcExchange(config_path="./config/default.toml")

# Synchronous usage
response = oidc.handle_request_sync(method, url, headers, body)
```

The `handle_request_sync` method takes the HTTP method, URL, headers dict, and optional body string. It returns a response object with `status`, `headers`, and `body` attributes.

## Framework Integration

### FastAPI (ASGI)

```python
from fastapi import FastAPI
from oidc_exchange import OidcExchange

app = FastAPI()
oidc = OidcExchange(config_path="./config/default.toml")

app.mount("/oidc", oidc.as_asgi())
```

The `as_asgi()` method returns a standard ASGI application that can be mounted directly into any ASGI framework.

### Flask (WSGI)

```python
from flask import Flask
from werkzeug.middleware.dispatcher import DispatcherMiddleware
from oidc_exchange import OidcExchange

app = Flask(__name__)
oidc = OidcExchange(config_path="./config/default.toml")

app.wsgi_app = DispatcherMiddleware(app.wsgi_app, {
    "/oidc": oidc.as_wsgi(),
})
```

### Django

Add a catch-all view in your `urls.py`:

```python
# urls.py
from django.urls import re_path
from oidc_exchange import OidcExchange

oidc = OidcExchange(config_path="./config/default.toml")

def oidc_view(request, path=""):
    response = oidc.handle_request_sync(
        method=request.method,
        url=request.build_absolute_uri(),
        headers=dict(request.headers),
        body=request.body.decode("utf-8") if request.body else None,
    )

    from django.http import HttpResponse
    django_response = HttpResponse(
        content=response.body,
        status=response.status,
    )
    for key, value in response.headers.items():
        django_response[key] = value
    return django_response

urlpatterns = [
    re_path(r"^oidc/(?P<path>.*)$", oidc_view),
]
```

## Async Support

The `handle_request` method is async and runs the underlying Rust code in an executor, making it safe to call from async frameworks without blocking the event loop:

```python
import asyncio
from oidc_exchange import OidcExchange

oidc = OidcExchange(config_path="./config/default.toml")

async def handle(method, url, headers, body=None):
    response = await oidc.handle_request(method, url, headers, body)
    return response
```

This is what `as_asgi()` uses internally, so FastAPI and other async frameworks get non-blocking behavior automatically.

## Configuration

### File path

```python
oidc = OidcExchange(config_path="./config/default.toml")
```

### Inline TOML

```python
oidc = OidcExchange(config_toml="""
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[registration]
mode = "open"
""")
```

See the [Configuration guide](/guides/configuration) for all available options.
