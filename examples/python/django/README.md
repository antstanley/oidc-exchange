# Django + oidc-exchange

Minimal example integrating oidc-exchange into a Django project using a
catch-all view that delegates to `handle_request_sync`.

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

## Run

```bash
python manage.py runserver 8080
```

The OIDC endpoints are available at `http://localhost:8080/auth/`.
