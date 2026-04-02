"""Tests for OidcExchange ASGI adapter."""

import json
import subprocess
import tempfile
from pathlib import Path

import httpx
import pytest
import pytest_asyncio

from oidc_exchange import OidcExchange


@pytest.fixture(scope="session")
def test_key_path():
    """Generate an Ed25519 test key for the session."""
    key_path = Path(tempfile.gettempdir()) / "oidc-test-python-asgi-key.pem"
    subprocess.run(
        ["openssl", "genpkey", "-algorithm", "Ed25519", "-out", str(key_path)],
        check=True,
        capture_output=True,
    )
    yield str(key_path)
    key_path.unlink(missing_ok=True)


@pytest.fixture(scope="session")
def test_config(test_key_path):
    """Return a TOML config string for testing."""
    db_path = "/tmp/oidc-test-python-asgi.db"
    return f"""
[session_store]
type = "sqlite"
path = "{db_path}"

[key_manager]
type = "local"
key_path = "{test_key_path}"

[audit]
type = "noop"

[server]
issuer = "https://auth.test.com"
registration_mode = "open"
role = "exchange"

[telemetry]
enabled = false
"""


@pytest.mark.asyncio
async def test_asgi_health(test_config):
    """ASGI app responds to GET /health with status 200."""
    instance = OidcExchange(config_string=test_config)
    app = instance.asgi_app()
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://testserver") as client:
        response = await client.get("/health")
        assert response.status_code == 200


@pytest.mark.asyncio
async def test_asgi_jwks(test_config):
    """ASGI app responds to GET /keys with status 200 and a JSON body containing 'keys'."""
    instance = OidcExchange(config_string=test_config)
    app = instance.asgi_app()
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://testserver") as client:
        response = await client.get("/keys")
        assert response.status_code == 200
        body = response.json()
        assert "keys" in body
        assert isinstance(body["keys"], list)
