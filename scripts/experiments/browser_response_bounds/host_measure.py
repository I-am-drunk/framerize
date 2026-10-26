"""Compare pinned host implementations using one synthetic HTTPS response."""
import argparse
import asyncio
from functools import partial
import hashlib
import importlib
import json
from pathlib import Path
import sys
import tempfile
import time
from unittest.mock import patch


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


class OriginPage:
    def __init__(self, page, app, origin):
        self.page, self.app = page, app
        require(app.AI_START_JS.count(app.API) == 1, 'expected one target origin')
        self.source = app.AI_START_JS.replace(app.API, origin)
        self.calls = dict(start=0, poll=0, cancel=0)

    async def evaluate(self, script, args):
        names = {self.app.AI_START_JS: 'start', self.app.AI_POLL_JS: 'poll',
                 self.app.AI_CANCEL_JS: 'cancel'}
        require(script in names, 'unexpected browser operation')
        self.calls[names[script]] += 1
        return await self.page.evaluate(self.source if names[script] == 'start' else script, args)


async def measure(host, page, app, report, sampled):
    from memory import snapshot
    from stream_queue import check_body, sample_while_running
    report['memory']['before_request'] = await snapshot()
    samples, stop = [], asyncio.Event()
    sampler = asyncio.create_task(sample_while_running(stop, samples)) if sampled else None
    started = time.perf_counter()
    try:
        value = await host._run_job(page, '/exact', {}, 'synthetic-token',
                                   'synthetic-bearer', 'measure', None, 10)
    finally:
        elapsed = (time.perf_counter() - started) * 1000
        stop.set()
        if sampler is not None:
            await asyncio.wait_for(sampler, 2)
        report['request_samples'] = samples
    report['memory']['after_request'] = await snapshot()
    check_body(value)
    require(set(value) == {'status', 'usage', 'body'}, 'response shape changed')
    digest = hashlib.sha256()
    for offset in range(0, len(value['body']), 65536):
        digest.update(value['body'][offset:offset + 65536].encode('ascii'))
    report['measurement'] = dict(request_to_body_ms=elapsed,
        body_sha256=digest.hexdigest(), characters=len(value['body']), calls=page.calls)
    require(page.calls['start'] == page.calls['cancel'] == 1, 'request or cleanup repeated')
    require(not host.lifecycles['fixture'].activity.pending, 'request RPC remained owned')
    require(await page.page.evaluate('Object.keys(window.__aiJobs).length') == 0,
            'browser retained a response job')
    report['checks'].append('exact_body_and_request_cleanup')


async def account_case(browser, server, report, app, sampled):
    context = await browser.new_context(ignore_https_errors=True)
    context.set_default_timeout(5000)
    real_page = await context.new_page()
    await real_page.goto(server.origin)
    page = OriginPage(real_page, app, server.origin)
    report['production_start_sha256'] = hashlib.sha256(app.AI_START_JS.encode()).hexdigest()
    report['executed_start_sha256'] = hashlib.sha256(page.source.encode()).hexdigest()
    with tempfile.TemporaryDirectory() as directory, patch.object(app, 'ROOT', directory):
        host, life = app.Host(), app.AccountLifecycle()
        life.browser, life.phase = browser, 'resident'
        receipt = life.close_receipt = app.BrowserCloseReceipt(browser)
        account = app.Account(None, context, page, life)
        account.browser = browser
        host.accounts['fixture'], host.lifecycles['fixture'] = account, life
        try:
            await measure(host, page, app, report, sampled)
            require(server.paths == ['/exact'] and not server.errors,
                    'unexpected fixture request or handler failure')
        finally:
            await asyncio.wait_for(host.close(), 15)
        require(receipt.acknowledged and receipt.closed and life.phase == 'absent'
                and not browser.is_connected(), 'host did not confirm browser closure')
        require(not life.activity.activities and not life.activity.pending,
                'account activity survived browser close')
        report['checks'].append('host_browser_close_acknowledged')


async def exercise(browser_path, server, report, *, app, sampled, label):
    from playwright.async_api import async_playwright
    report.update(label=label, sampled=sampled,
                  loaded_app_sha256=hashlib.sha256(Path(app.__file__).read_bytes()).hexdigest())
    async with async_playwright() as playwright:
        browser = await playwright.firefox.launch(
            executable_path=str(browser_path), headless=True, timeout=15000,
            firefox_user_prefs={'dom.ipc.processCount': 1,
                                'dom.ipc.processCount.webIsolated': 1})
        try:
            report['browser_version'] = browser.version
            await account_case(browser, server, report, app, sampled)
        finally:
            if browser.is_connected():
                await asyncio.wait_for(browser.close(), 5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--label', choices=('full', 'ack'), required=True)
    parser.add_argument('--sampled', action='store_true')
    args = parser.parse_args()
    require(args.runtime.is_absolute(), 'runtime must be an explicit absolute path')
    sys.path.insert(0, str(args.runtime))
    app = importlib.import_module('framerize_host.app')
    require(Path(app.__file__).resolve() == args.runtime / 'framerize_host/app.py',
            'a different host implementation was imported')
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from probe import run
    result = asyncio.run(run(args.browser, partial(exercise, app=app,
        sampled=args.sampled, label=args.label)))
    result['scope'] = 'Pinned host response comparison; synthetic local HTTPS only.'
    print(json.dumps(result, sort_keys=True), flush=True)
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    sys.exit(main())
