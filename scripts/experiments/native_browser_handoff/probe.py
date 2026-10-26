"""Exercise SuppliedHost with a real browser and synthetic loopback peers."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / "runtime"),
               str(ROOT / "scripts/experiments/shared_context_isolation")]
from fixture import HandoffServer, Reply
from memory import snapshot, visible_processes
from framerize_host import app, supplied
from playwright.async_api import async_playwright


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


async def wait(event):
    await asyncio.wait_for(event.wait(), 8)


def request(server, case, account="a", body=b"fixture response", status=200, held=False):
    value = dict(cmd="send_supplied", version=1,
        account="00000000-0000-0000-0000-00000000000" + ("1" if account == "a" else "2"),
        user="user-" + account, device="device-" + account, bearer="bearer-" + account,
        cookie_header=f"session=login-{account}; __cuid=device-{account}",
        path="/ai/v3/responses/", wire=json.dumps({"castleToken": "token-" + case, "case": case}),
        expires_unix_ms=int(time.time() * 1000) + 60000,
        timeout_ms=10000, response_limit=8 * 1024**2, proxy=server.proxy_url)
    server.expected[case] = value
    server.replies[case] = Reply(body, status, held)
    return value


def response(value, body, status=200):
    require(value["cleanup_confirmed"] is True, "context closure was unconfirmed")
    require(value["response"] == dict(status=status, usage="synthetic-usage",
        content_type="text/event-stream", body=body.decode()), "response changed")


def drained(host):
    require(not host.jobs and not host.exchanges and not host.uncertain,
            "request retained jobs, exchanges or unclosed contexts")
    require(not host.browser.contexts, "browser retained a completed request context")


async def deliveries(host, server, report):
    body = ("a😀" * (8 * 1024**2 // 5)).encode() + b"a" * (8 * 1024**2 % 5)
    data = request(server, "exact", body=body)
    response(await host.handle(data), body)
    drained(host)
    report["checks"].append("exact_8_mib_utf8_body_token_cookies_and_headers_preserved_once")
    refusal = b'{"error":"synthetic refusal"}'
    data = request(server, "refusal", body=refusal, status=403)
    response(await host.handle(data), refusal, 403)
    drained(host)
    report["checks"].append("http_refusal_returned_without_retry_or_cross_context_cookies")
    data = request(server, "oversize", body=b"a" * 1025)
    data["response_limit"] = 1024
    value = await host.handle(data)
    require(value["cleanup_confirmed"] and value["response"]["status"] == -1
            and value["response"]["failure_phase"] == "body"
            and value["response"]["head_status"] == 200,
            "oversize response did not fail at the body boundary")
    drained(host)
    report["checks"].append("oversize_response_discarded_with_context_closed")


async def concurrent(host, server, report):
    first = request(server, "held-a", held=True)
    second = request(server, "held-b", "b", body=b"second account intact", held=True)
    entered, release = asyncio.Event(), asyncio.Event()
    real_close = supplied.close_context
    async def held_close(context):
        entered.set()
        await release.wait()
        await real_close(context)
    tasks = [asyncio.create_task(host.handle(data)) for data in (first, second)]
    try:
        await wait(server.replies["held-a"].started)
        await wait(server.replies["held-b"].started)
        require(len(host.browser.contexts) == 2, "concurrent requests did not own separate contexts")
        require(server.posts["held-a"] != server.posts["held-b"], "accounts shared a connection")
        report["memory"]["two_active_contexts"] = await snapshot()
        with patch.object(supplied, "close_context", held_close):
            tasks[0].cancel()
            await wait(entered)
            require(not tasks[0].done() and not tasks[1].done(), "cancellation released a request early")
            require(len(host.jobs) == 2, "cancellation lost the original request owner")
            release.set()
            cancelled = await asyncio.wait_for(tasks[0], 8)
        require(cancelled == dict(version=1, response=None, cleanup_confirmed=True),
                "cancelled request did not join its context")
        await wait(server.replies["held-a"].closed)
        require(not tasks[1].done() and len(host.browser.contexts) == 1,
                "cancelling one account disturbed its peer")
        server.replies["held-b"].release.set()
        response(await asyncio.wait_for(tasks[1], 8), b"second account intact")
        drained(host)
        report["checks"].append("concurrent_accounts_keep_separate_cookies_connections_and_close_ownership")
    finally:
        release.set()
        for reply in server.replies.values():
            reply.release.set()
        for task in tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


async def deadline(host, server, report):
    data = request(server, "deadline", held=True)
    data["timeout_ms"] = 2000
    task = asyncio.create_task(host.handle(data))
    await wait(server.replies["deadline"].started)
    value = await asyncio.wait_for(task, 8)
    require(value["cleanup_confirmed"] and (value["response"] is None
            or value["response"]["status"] == -1),
            f"deadline outcome: joined={value['cleanup_confirmed']}, "
            f"status={value['response']['status'] if value['response'] else None}")
    await wait(server.replies["deadline"].closed)
    drained(host)
    report["checks"].append("request_deadline_closes_and_joins_the_held_fetch")


async def exercise(browser_path, server, report, directory):
    launches = []
    async def launch(playwright, **options):
        options.pop("humanize")
        browser = await playwright.firefox.launch(**options)
        create = browser.new_context
        async def trusted_context(**kwargs):
            return await create(ignore_https_errors=True, **kwargs)
        browser.new_context = trusted_context
        launches.append(browser)
        return browser
    with patch.object(app, "ROOT", directory), \
         patch.object(supplied, "browser_runtime", lambda: (launch, async_playwright)), \
         patch.object(supplied, "prepared_memory_browser", lambda: str(browser_path)), \
         patch.object(supplied, "prepared_memory_addon", lambda: None):
        host = supplied.SuppliedHost()
        try:
            await host.prepare()
            report["browser_version"] = host.browser.version
            report["memory"]["browser_ready"] = await snapshot()
            await deliveries(host, server, report)
            await concurrent(host, server, report)
            await deadline(host, server, report)
        finally:
            await asyncio.wait_for(host.close(), 15)
        require(len(launches) == 1 and host.receipt.acknowledged and host.receipt.closed
                and not host.browser.is_connected(), "shared browser close was unconfirmed")
        require(set(server.posts) == set(server.expected) and not server.errors,
                "missing, repeated or invalid fixture requests")
        report.update(launches=len(launches), posts=len(server.posts),
                      documents=server.documents, tunnels=server.tunnels,
                      blocked_other_tunnels=server.denied)
        report["checks"].append("one_shared_browser_all_contexts_and_browser_shutdown_acknowledged")


async def run(browser_path):
    for kind in ("net", "pid"):
        parent = os.environ.get("FIXTURE_PARENT_" + kind.upper())
        require(parent and os.readlink("/proc/self/ns/" + kind) != parent,
                "private network and PID namespaces are required")
    baseline = visible_processes()
    report = dict(checks=[], memory={}, status="failed", cleanup_joined=False)
    failure = None
    with tempfile.TemporaryDirectory(prefix="native-browser-handoff-") as directory:
        server = HandoffServer()
        await server.start(directory)
        try:
            await asyncio.wait_for(exercise(browser_path, server, report, directory), 50)
        except Exception as error:
            failure = type(error).__name__ + ": " + str(error)
        finally:
            await server.close()
    remaining = None
    for _ in range(20):
        try:
            remaining = {pid: stamp for pid, stamp in visible_processes().items()
                         if baseline.get(pid) != stamp}
        except OSError:
            remaining = None
        if remaining == {}:
            break
        await asyncio.sleep(0.1)
    report["cleanup_joined"] = remaining == {}
    if remaining != {}:
        failure = "browser descendants remained or could not be checked"
    report.update(status="failed" if failure else "passed", failure=failure,
        scope="Local supplied-token host with exact production origins and synthetic identities.")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", type=Path, required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser))
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result["status"] == "passed" else 1)
