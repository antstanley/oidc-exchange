# oidc-exchange

[![PyPI](https://img.shields.io/pypi/v/oidc-exchange)](https://pypi.org/project/oidc-exchange/)
[![Python](https://img.shields.io/pypi/pyversions/oidc-exchange)](https://pypi.org/project/oidc-exchange/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/antstanley/oidc-exchange/blob/main/LICENSE)

Python binding for [**oidc-exchange**](https://github.com/antstanley/oidc-exchange) — a Rust service that validates ID tokens from third-party OIDC providers (Google, Apple, …) and exchanges them for self-issued access and refresh tokens.

The service is embedded in-process as a native extension (built with [PyO3](https://pyo3.rs) + [maturin](https://www.maturin.rs)). Handle requests synchronously or with `async`, or mount the built-in **ASGI**/**WSGI** apps in FastAPI, Starlette, Flask, or Django.

## Install

```bash
pip install oidc-exchange
```

Ships as an `abi3` wheel — one wheel per platform works on **Python 3.10+**: `manylinux_2_28` x86_64/aarch64, `win_amd64`, and `macosx_11_0_arm64`. An sdist is published alongside for other platforms (needs a Rust toolchain to build).

## Usage

### ASGI (FastAPI / Starlette)

```python
from fastapi import FastAPI
from oidc_exchange import OidcExchange

oidc = OidcExchange(config="./config.toml")
app = FastAPI()
app.mount("/auth", oidc.asgi_app())
```

### WSGI (Flask / Django)

```python
from oidc_exchange import OidcExchange

oidc = OidcExchange(config_string="""
[server]
issuer = "https://auth.example.com"
…
""")
application = oidc.wsgi_app()
```

### Direct request handling

```python
from urllib.parse import urlencode

resp = oidc.handle_request_sync({
    "method": "POST",
    "path": "/token",
    "headers": {"content-type": "application/x-www-form-urlencoded"},
    "body": urlencode({
        "grant_type": "authorization_code",
        "code": "abc123",
        "redirect_uri": "https://app.example.com/callback",
        "provider": "google",
    }).encode(),
})
# resp -> {"status": 200, "headers": {...}, "body": b"…"}

# or await the async variant (runs the blocking call in the default executor):
resp = await oidc.handle_request(request)
```

### API

```python
class OidcExchange:
    def __init__(self, *, config: str | None = None, config_string: str | None = None) -> None: ...
    def handle_request_sync(self, request: dict) -> dict: ...
    async def handle_request(self, request: dict) -> dict: ...
    def asgi_app(self) -> Any: ...   # mountable ASGI application
    def wsgi_app(self) -> Any: ...   # mountable WSGI application
    def shutdown(self) -> None: ...
```

A request `dict` is `{"method", "path", "headers": dict[str, str], "body": bytes}`; the response is `{"status", "headers": dict[str, str], "body": bytes}`. The full service is exposed — `/token`, `/revoke`, `/keys`, `/.well-known/openid-configuration`, `/health`, and the internal admin API.

## Framework examples

See the main repo's [Python examples](https://github.com/antstanley/oidc-exchange/tree/main/examples/python): FastAPI, Flask, Django.

## Configuration

TOML config — providers, token TTLs, registration policy, key management, and storage. See the [configuration guide](https://github.com/antstanley/oidc-exchange#configuration).

## Links

- [Repository & full docs](https://github.com/antstanley/oidc-exchange)
- [Why oidc-exchange?](https://github.com/antstanley/oidc-exchange#why-oidc-exchange)

Published to PyPI via OIDC trusted publishing. MIT licensed.
