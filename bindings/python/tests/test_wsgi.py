"""Tests for OidcExchange WSGI adapter."""

import io
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest
from oidc_exchange import OidcExchange
from werkzeug.test import Client


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


def test_wsgi_health(test_config):
    """WSGI app responds to GET /health with status 200."""
    instance = OidcExchange(config_string=test_config)
    app = instance.wsgi_app()
    client = Client(app)
    response = client.get("/health")
    assert response.status_code == 200


@pytest.mark.parametrize(("length", "expected"), [(3, 404), (4, 404), (5, 413)])
def test_wsgi_body_limit_boundary(test_config, length, expected):
    """WSGI accepts at/below the cap and rejects one byte above without reading it."""
    from oidc_exchange._wsgi import make_wsgi_app

    instance = OidcExchange(config_string=test_config)
    app = make_wsgi_app(instance, max_request_body_bytes=4)
    client = Client(app)
    response = client.post("/nonexistent", data=b"x" * length)
    assert response.status_code == expected


@pytest.mark.parametrize(("length", "expected"), [("invalid", 400), (str(sys.maxsize), 413)])
def test_wsgi_malformed_and_huge_content_lengths(test_config, length, expected):
    """Malformed and huge lengths return typed HTTP errors without reading the stream."""
    from oidc_exchange._wsgi import make_wsgi_app

    instance = OidcExchange(config_string=test_config)
    app = make_wsgi_app(instance, max_request_body_bytes=4)
    status = []
    result = app(
        {
            "REQUEST_METHOD": "POST",
            "PATH_INFO": "/",
            "CONTENT_LENGTH": length,
            "wsgi.input": io.BytesIO(b"unused"),
        },
        lambda line, _headers: status.append(line),
    )
    assert list(result) == [b""]
    assert status[0].startswith(str(expected))


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
