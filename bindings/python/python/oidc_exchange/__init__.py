from oidc_exchange._oidc_exchange import OidcExchange as _OidcExchange

try:
    from oidc_exchange._asgi import make_asgi_app
except ImportError:
    make_asgi_app = None

try:
    from oidc_exchange._wsgi import make_wsgi_app
except ImportError:
    make_wsgi_app = None


class OidcExchange:
    def __init__(self, *, config=None, config_string=None):
        self._inner = _OidcExchange(config=config, config_string=config_string)

    def handle_request_sync(self, request):
        return self._inner.handle_request_sync(request)

    async def handle_request(self, request):
        import asyncio
        loop = asyncio.get_event_loop()
        return await loop.run_in_executor(None, self._inner.handle_request_sync, request)

    def asgi_app(self):
        if make_asgi_app is None:
            raise NotImplementedError("ASGI adapter will be implemented in Task 4.2")
        return make_asgi_app(self)

    def wsgi_app(self):
        if make_wsgi_app is None:
            raise NotImplementedError("WSGI adapter will be implemented in Task 4.2")
        return make_wsgi_app(self)

    def shutdown(self):
        self._inner.shutdown()


__all__ = ["OidcExchange"]
