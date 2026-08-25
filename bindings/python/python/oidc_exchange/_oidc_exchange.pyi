"""Type stub for the compiled native extension.

``oidc_exchange._oidc_exchange`` is built by maturin from the Rust ffi crate (see
``../../src/lib.rs``); the compiled module ships no inline types. This stub is the
typed surface pyright and downstream consumers resolve, whether or not the ``.so``
has been built — so the pure-Python sources stay strictly type-checked without a
``maturin develop`` step first.
"""

from typing import Any

class OidcExchange:
    """Native OIDC-Exchange instance wrapped by ``oidc_exchange.OidcExchange``."""

    def __init__(self, *, config: str | None = ..., config_string: str | None = ...) -> None: ...
    def handle_request_sync(self, request: dict[str, Any]) -> dict[str, Any]: ...
    def limits(self) -> int: ...
    def shutdown(self) -> None: ...
