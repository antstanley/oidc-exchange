"""Tests for OidcExchange WSGI adapter."""

import json
import subprocess
import tempfile
from pathlib import Path

import pytest
from werkzeug.test import Client

from oidc_exchange import OidcExchange


@pytest.fixture(scope="session")
def test_key_path():
    """Generate an Ed25519 test key for the session."""
    key_path = Path(tempfile.gettempdir()) / "oidc-test-python-wsgi-key.pem"
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
    db_path = "/tmp/oidc-test-python-wsgi.db"
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


def test_wsgi_health(test_config):
    """WSGI app responds to GET /health with status 200."""
    instance = OidcExchange(config_string=test_config)
    app = instance.wsgi_app()
    client = Client(app)
    response = client.get("/health")
    assert response.status_code == 200


def test_wsgi_jwks(test_config):
    """WSGI app responds to GET /keys with status 200 and a JSON body containing 'keys'."""
    instance = OidcExchange(config_string=test_config)
    app = instance.wsgi_app()
    client = Client(app)
    response = client.get("/keys")
    assert response.status_code == 200
    body = json.loads(response.data)
    assert "keys" in body
    assert isinstance(body["keys"], list)
