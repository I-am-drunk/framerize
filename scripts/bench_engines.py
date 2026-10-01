#!/usr/bin/env python3
"""Engine benchmark for framerize per-account sessions.
For each engine: launch -> framer.com/login -> wait for castlePublishableAPIKey
-> mint Castle token (v3 shape check) -> measure RSS of the whole process tree.
For chromium we additionally measure the per-context delta (shared instance)."""
import json, sys, time
import psutil
from playwright.sync_api import sync_playwright

CHS = "/workspace/chrome-headless-shell-linux64/chrome-headless-shell"
MINT = """(async () => {
  for (let i = 0; i < 3; i++) {
    try {
      const m = await import("https://app.framerstatic.com/chunk-5LKSDTQ5.mjs?cb=" + Date.now() + "_" + i);
      const t = await m.a();
      if (typeof t === "string" && t.length > 4000 && t.charAt(8) === "|") return "V3 " + t.length + " chars";
      if (typeof t === "string") return "V2-FALLBACK " + t.length + " chars";
    } catch (e) {}
    await new Promise(r => setTimeout(r, 1200));
  }
  return "FAIL";
})()"""

def rss_mb(marker):
    """Sum RSS of every process whose cmdline contains marker."""
    total = 0
    for p in psutil.process_iter(["cmdline"]):
        try:
            if p.info["cmdline"] and any(marker in a for a in p.info["cmdline"]):
                total += p.memory_info().rss
        except psutil.Error:
            pass
    return round(total / 1048576, 1)

def run():
    results = {}
    pw = sync_playwright().start()

    def bench(browser, name, marker):
        r = {"engine": name}
        r["rss_launch_mb"] = rss_mb(marker)
        ctx = browser.new_context()
        page = ctx.new_page()
        t0 = time.time()
        try:
            page.goto("https://framer.com/login", wait_until="domcontentloaded", timeout=45000)
            page.wait_for_function("typeof window.castlePublishableAPIKey !== 'undefined'", timeout=20000)
            r["load_s"] = round(time.time() - t0, 1)
            t0 = time.time()
            r["castle"] = page.evaluate(MINT)
            r["mint_s"] = round(time.time() - t0, 1)
        except Exception as e:
            r["error"] = str(e)[:200]
        r["rss_1acct_mb"] = rss_mb(marker)
        return browser, ctx, r

    # --- 1. chrome-headless-shell: per-instance cost + per-context delta ---
    b = pw.chromium.launch(executable_path=CHS, args=["--no-sandbox", "--disable-dev-shm-usage",
                                                      "--disable-blink-features=AutomationControlled",
                                                      "--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36"])
    marker = "chrome-headless-shell"
    b, ctx1, r = bench(b, "chrome-headless-shell", marker)
    # two more contexts (= two more accounts) to get the shared-instance delta
    extra = []
    for _ in range(2):
        c = b.new_context()
        pg = c.new_page()
        try:
            pg.goto("https://framer.com/login", wait_until="domcontentloaded", timeout=45000)
        except Exception:
            pass
        extra.append(c)
    time.sleep(3)
    r["rss_3acct_mb"] = rss_mb(marker)
    if r.get("rss_1acct_mb", -1) > 0 and r["rss_3acct_mb"] > 0:
        r["per_extra_context_mb"] = round((r["rss_3acct_mb"] - r["rss_1acct_mb"]) / 2, 1)
    results["headless_shell"] = r
    b.close()
    time.sleep(2)

    # --- 2. WebKit (playwright build): per-instance cost ---
    try:
        b = pw.webkit.launch()
        b, _, r = bench(b, "webkit (playwright build)", "MiniBrowser")
        results["webkit"] = r
        b.close()
    except Exception as e:
        results["webkit"] = {"engine": "webkit", "error": str(e)[:200]}
    time.sleep(2)

    pw.stop()
    print(json.dumps(results, indent=1))

if __name__ == "__main__":
    run()
