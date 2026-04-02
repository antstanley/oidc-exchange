# Flask + oidc-exchange

Minimal example mounting oidc-exchange as a WSGI sub-application under `/auth`
using Werkzeug's `DispatcherMiddleware`.

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

## Run

```bash
python app.py
```

The OIDC endpoints are available at `http://localhost:5000/auth/`.
