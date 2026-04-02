# FastAPI + oidc-exchange

Minimal example mounting oidc-exchange as an ASGI sub-application under `/auth`.

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

## Run

```bash
uvicorn main:app --reload --port 8080
```

The OIDC endpoints are available at `http://localhost:8080/auth/`.
