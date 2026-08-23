import asyncio
import importlib.util
import io
import json
import os
import sys
from typing import Any

shape, config, artifact, variant = sys.argv[1:]
if not os.path.isfile(artifact) or __import__("time").time() - os.path.getmtime(artifact) > 900:
    raise RuntimeError(f"fresh PyO3 artifact missing or stale: {artifact}")
spec = importlib.util.spec_from_file_location("oidc_exchange._oidc_exchange", artifact)
if spec is None or spec.loader is None: raise RuntimeError("cannot load PyO3 artifact")
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
sys.modules["oidc_exchange._oidc_exchange"] = module
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bindings", "python", "python"))
from oidc_exchange import OidcExchange
from oidc_exchange._asgi import make_asgi_app
from oidc_exchange._wsgi import make_wsgi_app

def body(f: dict[str, Any]) -> bytes: return b"x" * f["bodyLength"]
def decoded(value: str) -> str:
    from urllib.parse import unquote
    return unquote(value or "/")
def parsed(fid: str, status: int, data: bytes) -> dict[str, Any]:
    if data:
        try: return {"id": fid, **json.loads(data), "executed": True}
        except json.JSONDecodeError: pass
    return {"id": fid, "status": status, "executed": True}

async def asgi(app: Any, f: dict[str, Any]) -> dict[str, Any]:
    messages=[{"type":"http.request","body":body(f),"more_body":False}]; sent=[]
    async def receive(): return messages.pop(0)
    async def send(message): sent.append(message)
    scope={"type":"http","method":f["method"],"path":decoded(f["rawPath"]),"query_string":(f.get("query") or "").encode("latin-1"),"headers":[(h["name"].encode("latin-1"),h["value"].encode("latin-1")) for h in f["headers"]]}
    if variant=="faithful": scope["raw_path"]=f["rawPath"].encode("latin-1")
    await app(scope,receive,send)
    status=next(m["status"] for m in sent if m["type"]=="http.response.start")
    data=b"".join(m.get("body",b"") for m in sent if m["type"]=="http.response.body")
    return parsed(f["id"],status,data)

def wsgi(app: Any, f: dict[str, Any]) -> dict[str, Any]:
    captured={}
    def start_response(status,headers): captured.update(status=int(status.split()[0]),headers=headers)
    content=next((h["value"] for h in f["headers"] if h["name"].lower()=="content-length"),str(f["bodyLength"]))
    environ={"REQUEST_METHOD":f["method"],"PATH_INFO":decoded(f["rawPath"]),"QUERY_STRING":f.get("query") or "","wsgi.input":io.BytesIO(body(f)),"CONTENT_LENGTH":content}
    if variant=="faithful":
        environ["RAW_URI"]=f["rawPath"]+(("?"+f["query"]) if f.get("query") else "")
        environ["oidc_exchange.headers"]=[(h["name"],h["value"]) for h in f["headers"]]
    else:
        for h in f["headers"]:
            if h["name"].lower() not in ("content-length","content-type"): environ["HTTP_"+h["name"].upper().replace("-","_")]=h["value"]
    data=b"".join(app(environ,start_response))
    return parsed(f["id"],captured["status"],data)

async def main():
    oidc=OidcExchange(config=config); app=make_asgi_app(oidc) if shape=="asgi" else make_wsgi_app(oidc)
    for line in sys.stdin:
        f=json.loads(line); out=await asgi(app,f) if shape=="asgi" else wsgi(app,f); print(json.dumps(out),flush=True)
asyncio.run(main())
