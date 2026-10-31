# Framerize (browser-based fallback)

This branch keeps the older browser pool. It is a fallback. Use it if the current method on `main` breaks.

For how the service works, read the `main` README. This file only covers what is different here.

## What this branch is

`main` mints and sends inside each account's signed-in session. That is the method Castle accepts, verified 2026-10-31 with two live requests that returned HTTP 200.

This branch keeps the earlier design: a pool of browser sessions, one per account, held open by the daemon. It uses more memory because every account keeps a browser alive. It is here so you can go back to a known working setup if the current path stops working.

## When to use it

Use `main` first. Come back to this branch only if `main` stops passing Castle and you need a version that worked before.

## Running it

```bash
git checkout browser-based
./scripts/linux/install-camoufox.sh
./scripts/linux/start-daemon.sh
framerize login
```

A source checkout does not deploy or restart the running service. Build and deploy this branch the same way you would any other.

## Legal note

Framer's endpoints are private and undocumented. Automating them likely violates Framer's Terms of Service. Use this only for accounts you own, at your own risk.
