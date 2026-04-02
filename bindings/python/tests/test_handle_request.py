"""Tests for OidcExchange handle_request methods."""

import json
import subprocess
import tempfile
from pathlib import Path

import pytest

from oidc_exchange import OidcExchange


@pytest.fixture(scope="session")
def test_key_path():
    """Generate an Ed25519 test key for the session."""
    key_path = Path(tempfile.gettempdir()) / "oidc-test-python-key.pem"
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
    db_path = "/tmp/oidc-test-python.db"
    return f"""
[server]
issuer = "https://auth.test.com"
role = "exchange"

[registration]
mode = "open"

[repository]
adapter = "sqlite"

[repository.sqlite]
path = "{db_path}"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "{test_key_path}"
algorithm = "EdDSA"
kid = "test-key-1"

[audit]
adapter = "noop"

[telemetry]
enabled = false
"""


def test_create_instance(test_config):
    """OidcExchange can be instantiated with a config string."""
    instance = OidcExchange(config_string=test_config)
    assert instance is not None


def test_missing_config():
    """OidcExchange raises an exception when no config is provided."""
    with pytest.raises(Exception):
        OidcExchange()


def test_health_endpoint(test_config):
    """GET /health returns status 200."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync({
        "method": "GET",
        "path": "/health",
        "headers": {},
    })
    assert response["status"] == 200


def test_jwks_endpoint(test_config):
    """GET /keys returns status 200 with a JSON body containing a 'keys' array."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync({
        "method": "GET",
        "path": "/keys",
        "headers": {},
    })
    assert response["status"] == 200
    body = json.loads(response["body"])
    assert "keys" in body
    assert isinstance(body["keys"], list)


def test_openid_discovery(test_config):
    """GET /.well-known/openid-configuration returns the correct issuer."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync({
        "method": "GET",
        "path": "/.well-known/openid-configuration",
        "headers": {},
    })
    assert response["status"] == 200
    body = json.loads(response["body"])
    assert body["issuer"] == "https://auth.test.com"


def test_unknown_route(test_config):
    """GET /nonexistent returns status 404."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync({
        "method": "GET",
        "path": "/nonexistent",
        "headers": {},
    })
    assert response["status"] == 404


@pytest.mark.asyncio
async def test_async_health(test_config):
    """Async handle_request for GET /health returns status 200."""
    instance = OidcExchange(config_string=test_config)
    response = await instance.handle_request({
        "method": "GET",
        "path": "/health",
        "headers": {},
    })
    assert response["status"] == 200
