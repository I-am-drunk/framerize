"""Compare two synthetic accounts in shared and separate loopback browsers."""
import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
import statistics
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from memory import snapshot, visible_processes
from server import Server
from playwright.async_api import async_playwright


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


async def state(page, value=None):
    return await page.evaluate("value => fixtureState(value)", value)


def expected(value):
    return [value, value, "fixture_session=" + value, value, value]


async def exercise(args, server, report):
    browsers, contexts, pages = [], [], []
    async with async_playwright() as playwright:
        try:
            for index, marker in enumerate(("fixture-a", "fixture-b")):
                if not browsers or args.mode == "separate":
                    browser = await playwright.firefox.launch(
                        executable_path=str(args.browser), headless=True, timeout=15000,
                        firefox_user_prefs={"dom.ipc.processCount": 1,
                                            "dom.ipc.processCount.webIsolated": 1,
                                            "browser.cache.disk.enable": False,
                                            "browser.cache.memory.enable": False})
                    browsers.append(browser)
                    report["browser_version"] = browser.version
                context = await browsers[-1].new_context(ignore_https_errors=True)
                contexts.append(context)
                context.set_default_timeout(5000)
                page = await context.new_page()
                pages.append(page)
                await page.goto(server.origin)
                await page.evaluate("() => {\n" + (HERE / "fixture.js").read_text() + "\n}")
                require(await state(page) == [None, None, "", None, None], "new context inherited storage")
                require(await state(page, marker) == expected(marker), "storage setup failed")
                await asyncio.sleep(0.1)
                report["memory"][f"accounts_{index + 1}"] = await snapshot()
            require(await state(pages[0]) == expected("fixture-a"), "second account altered first")
            await switch_and_close(server, contexts, pages, report)
        finally:
            failures = []
            for context in reversed(contexts):
                try:
                    await asyncio.wait_for(context.close(), 5)
                except Exception as error:
                    failures.append(type(error).__name__)
            for browser in reversed(browsers):
                try:
                    await asyncio.wait_for(browser.close(), 5)
                except Exception as error:
                    failures.append(type(error).__name__)
            require(not failures, "browser cleanup failed: " + ",".join(failures))


async def switch_and_close(server, contexts, pages, report):
    a, b = pages
    timings, connections = [], {}
    for index in range(12):
        page, marker = (a, "fixture-a") if index % 2 == 0 else (b, "fixture-b")
        started = time.monotonic()
        reply = await page.evaluate("async () => (await fetch('/echo')).json()")
        timings.append((time.monotonic() - started) * 1000)
        require(reply["cookie"] == "fixture_session=" + marker, "cross-account cookie")
        owner = connections.setdefault(reply["connection"], marker)
        require(owner == marker, "HTTPS connection reused across accounts")
    report["switch_roundtrip_ms"] = dict(count=len(timings), median=statistics.median(timings),
                                          maximum=max(timings))
    await a.evaluate("() => beginHeld('/hold')")
    await a.wait_for_function("window.firstChunk === 'first'")
    reply = await b.evaluate("async () => (await fetch('/echo')).json()")
    require(reply["cookie"] == "fixture_session=fixture-b", "held stream changed other account")
    require(await state(a) == expected("fixture-a"), "held stream lost account state")
    report["memory"]["during_stream"] = await snapshot()
    await b.evaluate("() => beginHeld('/cancel')")
    await b.wait_for_function("window.firstChunk === 'first'")
    await asyncio.wait_for(server.cancel_started.wait(), 3)
    await asyncio.wait_for(contexts.pop().close(), 5)
    await asyncio.wait_for(server.cancel_closed.wait(), 3)
    require(await state(a) == expected("fixture-a"), "closing other account changed storage")
    server.release.set()
    require(await a.evaluate("() => window.heldResult") == "first-last", "other account broke stream")
    owners = {}
    for row in server.records:
        if row["cookie"]:
            previous = owners.setdefault(row["connection"], row["cookie"])
            require(previous == row["cookie"], "connection crossed account ownership")
    require(not server.errors, "fixture server error: " + "; ".join(server.errors))
    report["checks"] = ["fresh_storage", "cookies", "local_storage", "session_storage",
                        "indexed_db", "cache_storage", "connection_ownership",
                        "switch_during_stream", "close_other_account_during_stream"]
    report["https_connections"] = len(owners)
    report["memory"]["after_second_context_close"] = await snapshot()


async def run(args):
    for kind in ("net", "pid"):
        parent = os.environ.get("FIXTURE_PARENT_" + kind.upper())
        require(parent and os.readlink("/proc/self/ns/" + kind) != parent,
                "private network and PID namespaces are required")
    baseline = visible_processes()
    report = dict(mode=args.mode, memory={}, browser_version=None)
    report["memory"]["before_browser"] = await snapshot()
    with tempfile.TemporaryDirectory(prefix="context-isolation-") as directory:
        server = Server()
        await server.start(directory)
        try:
            await asyncio.wait_for(exercise(args, server, report), 45)
        except Exception as error:
            report.update(status="failed", failure_type=type(error).__name__)
            report["memory"]["failure"] = await snapshot()
            print(json.dumps(report, sort_keys=True), flush=True)
            raise
        finally:
            await server.close()
    for _ in range(20):
        remaining = {pid: stamp for pid, stamp in visible_processes().items()
                     if baseline.get(pid) != stamp}
        if not remaining:
            break
        await asyncio.sleep(0.1)
    require(not remaining, "browser descendants remained after cleanup")
    report["memory"]["after_browser_close"] = await snapshot()
    report["cleanup_joined"] = True
    report["versions"] = {name: importlib.metadata.version(name)
                          for name in ("playwright", "camoufox")}
    report["sources"] = {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest()
                         for name in ("probe.py", "server.py", "memory.py", "fixture.js")}
    report["status"] = "passed"
    report["scope"] = "Local synthetic storage/connection isolation; no fingerprint, live acceptance or production RAM claim."
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", type=Path, required=True)
    parser.add_argument("--mode", choices=("shared", "separate"), required=True)
    arguments = parser.parse_args()
    print(json.dumps(asyncio.run(run(arguments)), sort_keys=True))
