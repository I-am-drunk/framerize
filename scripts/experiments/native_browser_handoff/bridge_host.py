"""Synthetic local peer for the Rust BrowserHost pipe integration test."""
import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from probe import HandoffServer, Reply, app, supplied, async_playwright, require


class LocalHost(supplied.SuppliedHost):
    def __init__(self, server, receipts):
        super().__init__()
        self.server, self.receipts, self.results = server, receipts, []

    async def handle(self, req):
        if req.get("cmd") != "send_supplied":
            return await super().handle(req)
        value = json.loads(req["wire"])
        case = value["case"]
        require(case in ("bridge-exact", "bridge-held"), "unexpected synthetic case")
        require(value["castleToken"] == "fixture-native-output" and
                req["cookie_header"] == "session=fixture-login; __cuid=fixture-device" and
                req["bearer"] == "fixture-bearer", "unexpected synthetic identity")
        req["proxy"] = self.server.proxy_url
        self.server.expected[case] = req
        reply = self.server.replies[case] = Reply(b"z" * 524288, held=case == "bridge-held")
        async def entered():
            await reply.started.wait()
            (self.receipts / case).write_text("started\n")
        marker = asyncio.create_task(entered())
        try:
            result = await super().handle(req)
            self.results.append(result["cleanup_confirmed"])
            return result
        finally:
            marker.cancel()
            await asyncio.gather(marker, return_exceptions=True)


async def main():
    for kind in ("net", "pid"):
        parent = os.environ.get("FIXTURE_PARENT_" + kind.upper())
        require(parent and os.readlink("/proc/self/ns/" + kind) != parent,
                "private network and PID namespaces are required")
    receipts = Path(os.environ["FRAMERIZE_STATE"]).parent
    receipts.mkdir(parents=True, exist_ok=True)
    async def launch(playwright, **options):
        options.pop("humanize")
        browser = await playwright.firefox.launch(**options)
        create = browser.new_context
        async def trusted_context(**kwargs):
            return await create(ignore_https_errors=True, **kwargs)
        browser.new_context = trusted_context
        return browser
    with tempfile.TemporaryDirectory(prefix="handoff-pipe-") as directory:
        server = HandoffServer()
        await server.start(directory)
        with patch.object(app, "ROOT", directory), \
             patch.object(supplied, "browser_runtime", lambda: (launch, async_playwright)), \
             patch.object(supplied, "prepared_memory_browser", lambda: os.environ["FIXTURE_BROWSER"]), \
             patch.object(supplied, "prepared_memory_addon", lambda: None):
            host = LocalHost(server, receipts)
            try:
                await host.loop()
            finally:
                await server.close()
            result = dict(posts=sorted(server.posts), results=host.results,
                errors=server.errors, contexts=len(host.browser.contexts),
                browser_closed=host.receipt.closed and host.receipt.acknowledged,
                jobs=len(host.jobs), exchanges=len(host.exchanges), uncertain=len(host.uncertain))
            (receipts / "fixture-receipt.json").write_text(json.dumps(result))


if __name__ == "__main__":
    asyncio.run(main())
