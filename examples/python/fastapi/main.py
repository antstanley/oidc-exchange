from fastapi import FastAPI
from oidc_exchange import OidcExchange

app = FastAPI()
oidc = OidcExchange(config="../config.toml")
app.mount("/auth", oidc.asgi_app())

@app.get("/")
async def root():
    return {"message": "FastAPI app with oidc-exchange mounted at /auth"}
