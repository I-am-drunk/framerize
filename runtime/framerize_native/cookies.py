"""Validate native authority cookie snapshots without browser orchestration."""
LIMIT = 16 * 1024


def login_cookie(header):
    if (type(header) is not str or not header or len(header.encode()) > LIMIT
            or any(ord(c) < 32 or ord(c) > 126 for c in header)):
        raise ValueError("invalid private cookie snapshot")
    login = None
    allowed = "!#$%&'*+-.^_|~" + chr(96)
    for part in header.split(";"):
        name, separator, value = part.strip().partition("=")
        if (not separator or not name
                or any(not c.isalnum() and c not in allowed for c in name)
                or any(ord(c) < 33 or ord(c) in (34, 44, 92) for c in value)):
            raise ValueError("invalid private cookie snapshot")
        if name == "session":
            if login is not None or not value:
                raise ValueError("ambiguous private login")
            login = value
    if login is None:
        raise ValueError("missing private login")
    return login
