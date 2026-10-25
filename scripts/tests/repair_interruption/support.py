def require(ok, reason):
    if not ok:
        raise RuntimeError(reason)
