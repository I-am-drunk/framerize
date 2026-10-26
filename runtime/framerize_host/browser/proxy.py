"""Translate a configured proxy URL to Playwright's separate auth fields."""
from urllib.parse import unquote, urlsplit


def proxy_options(proxy):
    # Playwright accepts host:port as HTTP, but strips userinfo from server.
    # Keep unauthenticated inputs unchanged for its existing normalization.
    try:
        parsed = urlsplit(proxy)
        if not parsed.scheme or not parsed.netloc:
            parsed = urlsplit("http://" + proxy)
        if parsed.username is None:
            return {"server": proxy}
        if not parsed.hostname:
            raise ValueError("missing proxy host")
        server = parsed.netloc.rsplit("@", 1)[1]
        return {"server": f"{parsed.scheme}://{server}",
                "username": unquote(parsed.username),
                "password": unquote(parsed.password or "")}
    except ValueError:
        raise ValueError("invalid account proxy") from None
