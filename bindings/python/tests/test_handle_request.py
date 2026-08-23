"""Tests for OidcExchange handle_request methods."""

import http.server
import json
import subprocess
import tempfile
import threading
import time
from pathlib import Path

import pytest
from oidc_exchange import OidcExchange

# Regression test tuning for `test_handle_request_sync_releases_gil` below.
#
# The test points the `user_sync` webhook adapter at a local HTTP server that
# deliberately sleeps this long before responding, so a single
# `handle_request_sync(POST /internal/users)` call blocks the FFI thread for
# a known, generous duration. `handle_request` awaits the webhook delivery
# synchronously before returning (see `admin_create_user`), so the *only* way
# a second Python thread can make substantial progress while that wait is in
# flight is if `handle_request_sync` releases the GIL (`py.allow_threads`)
# around the call.
_GIL_TEST_WEBHOOK_DELAY_SECONDS = 0.3

# Timeout given to the webhook HTTP client, comfortably above the delay above
# so the delivery completes rather than aborting on its own timeout.
_GIL_TEST_WEBHOOK_CLIENT_TIMEOUT = "10s"

# Window used to calibrate how fast the counter thread runs when it has the
# GIL to itself, uncontended. The in-flight rate below is compared against
# this baseline rather than a fixed absolute count, so the assertion adapts
# to the speed of the machine running the test.
_GIL_TEST_BASELINE_WINDOW_SECONDS = 0.2

# The counter thread's rate while `handle_request_sync` is in flight must
# reach at least this fraction of its uncontended baseline rate to count as
# "the GIL was released". Empirically, a thread that merely leaks through the
# rare forced GIL handoff during a held-GIL blocking call (i.e. the pre-change
# behaviour) achieves well under 1% of the baseline rate, while a released GIL
# lets the counter run near-uncontended (order 90%+); 5% leaves an ample
# margin on both sides.
_GIL_TEST_MIN_RATE_FRACTION = 0.05

# Upper bound on how long we wait for background threads to stop, so the test
# cannot hang if something goes wrong.
_GIL_TEST_JOIN_TIMEOUT_SECONDS = 10.0


def _measure_counter_rate(window_seconds: float) -> float:
    """Run a tight Python counting loop alone for `window_seconds` and return
    its increments-per-second rate, to calibrate against machine speed."""
    counter = {"value": 0}
    stop = threading.Event()

    def bump_counter() -> None:
        while not stop.is_set():
            counter["value"] += 1

    thread = threading.Thread(target=bump_counter)
    thread.start()
    time.sleep(window_seconds)
    stop.set()
    thread.join(timeout=_GIL_TEST_JOIN_TIMEOUT_SECONDS)
    return counter["value"] / window_seconds


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
    response = instance.handle_request_sync(
        {
            "method": "GET",
            "path": "/health",
            "headers": {},
        }
    )
    assert response["status"] == 200


def test_jwks_endpoint(test_config):
    """GET /keys returns status 200 with a JSON body containing a 'keys' array."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync(
        {
            "method": "GET",
            "path": "/keys",
            "headers": {},
        }
    )
    assert response["status"] == 200
    body = json.loads(response["body"])
    assert "keys" in body
    assert isinstance(body["keys"], list)


def test_openid_discovery(test_config):
    """GET /.well-known/openid-configuration returns the correct issuer."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync(
        {
            "method": "GET",
            "path": "/.well-known/openid-configuration",
            "headers": {},
        }
    )
    assert response["status"] == 200
    body = json.loads(response["body"])
    assert body["issuer"] == "https://auth.test.com"


def test_unknown_route(test_config):
    """GET /nonexistent returns status 404."""
    instance = OidcExchange(config_string=test_config)
    response = instance.handle_request_sync(
        {
            "method": "GET",
            "path": "/nonexistent",
            "headers": {},
        }
    )
    assert response["status"] == 404


@pytest.mark.asyncio
async def test_async_health(test_config):
    """Async handle_request for GET /health returns status 200."""
    instance = OidcExchange(config_string=test_config)
    response = await instance.handle_request(
        {
            "method": "GET",
            "path": "/health",
            "headers": {},
        }
    )
    assert response["status"] == 200


def test_handle_request_sync_accepts_empty_path_without_assertion(test_config):
    """An empty path reaches the legacy FFI boundary and returns its typed error."""
    instance = OidcExchange(config_string=test_config)
    with pytest.raises(RuntimeError, match="REQUEST_BUILD_ERROR"):
        instance.handle_request_sync({"method": "GET", "path": ""})


@pytest.mark.parametrize("missing_field", ["method", "path"])
def test_handle_request_sync_missing_required_field_raises_key_error(test_config, missing_field):
    """Missing required fields retain the documented KeyError contract."""
    instance = OidcExchange(config_string=test_config)
    request = {"method": "GET", "path": "/health"}
    del request[missing_field]
    with pytest.raises(KeyError, match=missing_field):
        instance.handle_request_sync(request)


@pytest.mark.parametrize(
    ("request_data", "message"),
    [
        ({"method": 1, "path": "/health"}, "method.*string"),
        ({"method": "GET", "path": 1}, "path.*string"),
        ({"method": "GET", "path": "/health", "body": object()}, "body.*bytes or a string"),
        ({"method": "GET", "path": "/health", "headers": []}, "headers.*dictionary"),
    ],
)
def test_handle_request_sync_invalid_field_type_raises_value_error(
    test_config, request_data, message
):
    """Ill-typed direct inputs fail as ValueError rather than panicking or defaulting."""
    instance = OidcExchange(config_string=test_config)
    with pytest.raises(ValueError, match=message):
        instance.handle_request_sync(request_data)


def test_handle_request_sync_invalid_method_raises_runtime_error(test_config):
    """An errored FFI request (invalid HTTP method) still raises PyRuntimeError."""
    instance = OidcExchange(config_string=test_config)
    with pytest.raises(RuntimeError):
        instance.handle_request_sync(
            {
                # Not a valid HTTP method token (contains a space), so the FFI
                # layer's `http::Method::from_str` fails and `handle_request_sync`
                # must map the resulting `FfiError` to `PyRuntimeError` even
                # though the call is wrapped in `py.allow_threads`.
                "method": "BAD METHOD",
                "path": "/health",
                "headers": {},
            }
        )


class _SlowWebhookHandler(http.server.BaseHTTPRequestHandler):
    """Responds to a single POST after `_GIL_TEST_WEBHOOK_DELAY_SECONDS`."""

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler naming
        time.sleep(_GIL_TEST_WEBHOOK_DELAY_SECONDS)
        body = b"{}"
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        # Silence the default stderr access logging; the test doesn't need it.
        pass


def test_handle_request_sync_releases_gil():
    """A second Python thread makes progress at close to its uncontended rate
    while a single, deliberately slow `handle_request_sync` call is in
    flight, proving the blocking FFI call releases the GIL.

    Without `py.allow_threads` around the FFI call, the calling thread holds
    the GIL for effectively the entire duration of the blocking webhook wait
    inside `handle_request_sync`, so the counter thread's throughput collapses
    to a tiny fraction of its uncontended rate. This test fails (throughput
    far below `_GIL_TEST_MIN_RATE_FRACTION` of baseline) against the
    pre-change `lib.rs` and passes once `py.allow_threads` wraps the call.
    """
    baseline_rate = _measure_counter_rate(_GIL_TEST_BASELINE_WINDOW_SECONDS)
    assert baseline_rate > 0, "counter thread made no progress even uncontended"

    server = http.server.HTTPServer(("127.0.0.1", 0), _SlowWebhookHandler)
    port = server.server_address[1]
    server_thread = threading.Thread(target=server.handle_request)
    server_thread.start()

    db_path = Path(tempfile.gettempdir()) / "oidc-test-python-gil.db"
    db_path.unlink(missing_ok=True)
    admin_config = f"""
[server]
issuer = "https://auth.test.com"
role = "admin"

[repository]
adapter = "sqlite"

[repository.sqlite]
path = "{db_path}"

[user_sync]
enabled = true
adapter = "webhook"

[user_sync.webhook]
url = "http://127.0.0.1:{port}"
secret = "test-webhook-secret"
timeout = "{_GIL_TEST_WEBHOOK_CLIENT_TIMEOUT}"
retries = 0

[internal_api]
enabled = true
auth_method = "shared_secret"
shared_secret = "test-internal-secret"

[audit]
adapter = "noop"

[telemetry]
enabled = false
"""
    instance = OidcExchange(config_string=admin_config)

    counter = {"value": 0}
    stop = threading.Event()

    def bump_counter() -> None:
        while not stop.is_set():
            counter["value"] += 1

    counter_thread = threading.Thread(target=bump_counter)
    counter_thread.start()
    try:
        start = time.monotonic()
        response = instance.handle_request_sync(
            {
                "method": "POST",
                "path": "/internal/users",
                "headers": {
                    "authorization": "Bearer test-internal-secret",
                    "content-type": "application/json",
                },
                "body": json.dumps({"external_id": "gil-test-user", "provider": "test-provider"}),
            }
        )
        elapsed = time.monotonic() - start
        counter_during_call = counter["value"]
    finally:
        stop.set()
        counter_thread.join(timeout=_GIL_TEST_JOIN_TIMEOUT_SECONDS)
        server_thread.join(timeout=_GIL_TEST_JOIN_TIMEOUT_SECONDS)

    assert not counter_thread.is_alive(), "counter thread failed to stop in time"
    assert response["status"] == 201

    # Sanity check that the call actually went through the slow webhook path
    # (and wasn't, say, short-circuited by a config or routing error).
    assert elapsed >= _GIL_TEST_WEBHOOK_DELAY_SECONDS

    rate_during_call = counter_during_call / elapsed
    assert rate_during_call >= _GIL_TEST_MIN_RATE_FRACTION * baseline_rate, (
        f"counter thread ran at {rate_during_call:.0f}/s while the slow "
        f"handle_request_sync call was in flight, far below its uncontended "
        f"baseline of {baseline_rate:.0f}/s; the GIL was not released around "
        "the blocking FFI call"
    )
