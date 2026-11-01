#!/usr/bin/env python3
"""Bounded local-fixture benchmark. Never loads account profiles or vendor pages."""
import argparse
import asyncio
import json
import statistics
import time
from pathlib import Path

from camoufox.async_api import AsyncCamoufox, AsyncNewBrowser
from playwright.async_api import async_playwright


def memory():
    group = next(line[3:] for line in Path("/proc/self/cgroup").read_text().splitlines()
                 if line.startswith("0::"))
    root = Path("/sys/fs/cgroup") / group.lstrip("/")
    pss = 0
    processes = 0
    for file in root.rglob("cgroup.procs"):
        for pid in file.read_text().split():
            try:
                lines = (Path("/proc") / pid / "smaps_rollup").read_text().splitlines()
                pss += next(int(line.split()[1]) for line in lines if line.startswith("Pss:"))
                processes += 1
            except (OSError, StopIteration):
                pass
    available = next(int(line.split()[1]) * 1024 for line in Path("/proc/meminfo").read_text().splitlines()
                     if line.startswith("MemAvailable:"))
    return {"pss_mib": round(pss / 1024, 2), "processes": processes,
            "cgroup_mib": round(int((root / "memory.current").read_text()) / 1024**2, 2),
            "host_available_mib": round(available / 1024**2, 2)}


async def main(args):
    # A route fixture supplies a stable HTTP origin for cookies/localStorage.
    # Every page request is fulfilled locally, including unexpected URLs.
    html = '<html><input name="email"><script>window.fixtureReady=true</script></html>'
    managers, browsers, pages = [], [], []
    driver = None
    result = {"mode": args.mode, "requested_accounts": args.accounts, "samples": []}
    result["baseline"] = memory()
    started = time.monotonic()
    try:
        if args.mode == "shared":
            driver = await async_playwright().start()
        for i in range(args.accounts):
            before = memory()
            if before["cgroup_mib"] > args.max_memory_mib - 300 or before["host_available_mib"] < 2000:
                result["stopped_for_memory"] = True
                break
            options = dict(headless=True, humanize=False, timeout=30000,
                           firefox_user_prefs={
                               "dom.ipc.processCount": 1,
                               "dom.ipc.processCount.webIsolated": 1,
                               "browser.cache.memory.enable": False,
                               "browser.cache.disk.enable": False,
                               "browser.sessionhistory.max_entries": 2,
                           })
            if driver is None:
                manager = AsyncCamoufox(**options)
                browser = await manager.__aenter__()
                managers.append(manager)
            else:
                browser = await AsyncNewBrowser(driver, **options)
            browsers.append(browser)
            ctx = await browser.new_context()
            await ctx.route("**/*", lambda route: route.fulfill(status=200, content_type="text/html", body=html))
            page = await ctx.new_page()
            await page.goto("http://fixture.invalid/", wait_until="load")
            # Use the same key and origin to catch accidental storage sharing.
            await page.evaluate("(id) => {localStorage.setItem('account', id); document.cookie='account='+id}", str(i))
            pages.append(page)
            sample = {"accounts": len(pages), **memory()}
            result["samples"].append(sample)
            print(json.dumps({"progress": sample}), flush=True)
        result["startup_s"] = round(time.monotonic() - started, 3)
        for i, page in enumerate(pages):
            state = await page.evaluate("() => [localStorage.getItem('account'), document.cookie]")
            assert state == [str(i), "account=" + str(i)], "account storage crossed browser boundaries"
        times = []
        for _ in range(10):
            t0 = time.monotonic()
            await asyncio.gather(*(p.evaluate("() => 42") for p in pages))
            times.append((time.monotonic() - t0) * 1000)
        result["fixture_round_trip_p50_ms"] = round(statistics.median(times), 3)
        result["storage_isolated"] = True
        result["final"] = memory()
    finally:
        if managers:
            for manager in reversed(managers):
                await manager.__aexit__(None, None, None)
        else:
            for browser in reversed(browsers):
                await browser.close()
        if driver:
            await driver.stop()
    result["after_close"] = memory()
    if args.output:
        Path(args.output).write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["baseline", "shared"], required=True)
    parser.add_argument("--accounts", type=int, default=3)
    parser.add_argument("--max-memory-mib", type=int, default=1200)
    parser.add_argument("--output")
    asyncio.run(main(parser.parse_args()))
