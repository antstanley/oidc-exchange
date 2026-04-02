from flask import Flask
from werkzeug.middleware.dispatcher import DispatcherMiddleware
from oidc_exchange import OidcExchange

app = Flask(__name__)
oidc = OidcExchange(config="../config.toml")
app.wsgi_app = DispatcherMiddleware(app.wsgi_app, {"/auth": oidc.wsgi_app()})

@app.route("/")
def index():
    return "Flask app with oidc-exchange mounted at /auth"

if __name__ == "__main__":
    app.run(port=5000, debug=True)
