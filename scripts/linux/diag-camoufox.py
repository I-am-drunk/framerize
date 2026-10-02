#!/usr/bin/env python3
"""diag-camoufox.py — watch what framer.com/login actually does inside Camoufox.
Prints page JS errors, console errors, and whether Castle's global appears.
Run:  ~/.framerize/venv/bin/python scripts/linux/diag-camoufox.py
"""
from camoufox.sync_api import Camoufox

with Camoufox(headless=True) as b:
    p = b.new_page()
    p.on("pageerror", lambda e: print("PAGEERROR:", str(e)[:300], flush=True))
    p.on("console", lambda m: m.type in ("error", "warning") and print(f"CONSOLE-{m.type.upper()}:", m.text[:300], flush=True))
    p.goto("https://framer.com/login", wait_until="domcontentloaded", timeout=90000)
    for i in range(1, 7):
        p.wait_for_timeout(3000)
        state = p.evaluate("""({
            key: typeof window.castlePublishableAPIKey,
            scripts: document.scripts.length,
            title: document.title,
            url: location.href,
            hasLoginForm: !!document.querySelector('input[type=email], input[name=email]'),
        })""")
        print(f"t+{i*3}s:", state, flush=True)
        if state["key"] != "undefined":
            print("CASTLE KEY APPEARED — page is healthy in camoufox")
            break
    else:
        print("castle key NEVER appeared in 18s — the page's JS is broken in this engine")
