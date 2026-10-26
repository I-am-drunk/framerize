"""Copy account cookies only through acknowledged, serialized browser writes."""


async def set_account_cookie(host, account, acc, name, value):
    # The caller retains this whole operation with Host._rpc, including locks,
    # so cancellation cannot let a later cookie write overtake an older one.
    cookie = {"name": name, "value": value, "domain": ".framer.com", "path": "/",
              "secure": True, "httpOnly": True, "sameSite": "None"}
    async with acc.cookie_lock:
        await host._rpc(account, acc.ctx.add_cookies, [cookie])
        if name == "session":
            # New devices take this value. Existing initializations must finish
            # before their context can be updated and this operation acknowledged.
            acc.session_cookie = (name, value)
            for device in list(acc.dev_objs.values()):
                async with device.lock:
                    if device.ctx is not None:
                        await host._rpc(account, device.ctx.add_cookies, [cookie])
    return True
