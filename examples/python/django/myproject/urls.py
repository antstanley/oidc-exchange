import os
from django.http import HttpResponse, JsonResponse
from django.urls import path, re_path
from oidc_exchange import OidcExchange

oidc = OidcExchange(config=os.path.join(os.path.dirname(__file__), "..", "..", "config.toml"))


def index(request):
    return JsonResponse({"message": "Django app with oidc-exchange mounted at /auth"})


def oidc_view(request, oidc_path=""):
    headers = {}
    for key, value in request.META.items():
        if key.startswith("HTTP_"):
            header_name = key[5:].replace("_", "-").lower()
            headers[header_name] = value
    if "CONTENT_TYPE" in request.META:
        headers["content-type"] = request.META["CONTENT_TYPE"]
    if "CONTENT_LENGTH" in request.META:
        headers["content-length"] = request.META["CONTENT_LENGTH"]

    req_path = f"/{oidc_path}"
    if request.META.get("QUERY_STRING"):
        req_path = f"{req_path}?{request.META['QUERY_STRING']}"

    response = oidc.handle_request_sync({
        "method": request.method,
        "path": req_path,
        "headers": headers,
        "body": request.body,
    })

    django_response = HttpResponse(
        content=response["body"],
        status=response["status"],
    )
    for name, value in response["headers"].items():
        django_response[name] = value
    return django_response


urlpatterns = [
    path("", index),
    re_path(r"^auth/(?P<oidc_path>.*)$", oidc_view),
]
