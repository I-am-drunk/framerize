"""Exercise the integrated host response path against local HTTPS only."""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ack_probe import LossyPage, empty_jobs
from probe import require, run
from stream_queue import check_body
from framerize_host import app
from framerize_host.browser import scripts
from playwright.async_api import async_playwright


class HostPage:
    def __init__(self, page, source):
        self.page, self.source = page, source
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.reset()

    def reset(self, fault=None, hold_final=False):
        self.lossy = LossyPage(self.page, fault)
        self.hold_final = hold_final
        self.starts = self.polls = self.cancels = 0
        self.entered.clear()
        self.release.clear()

    async def evaluate(self, script, args):
        if script == app.AI_START_JS:
            self.starts += 1
            return await self.page.evaluate(self.source, args)
        if script == app.AI_POLL_JS:
            self.polls += 1
            value = await self.lossy.evaluate(script, args)
            if self.hold_final and value['kind'] == 'closed':
                self.entered.set()
                await self.release.wait()
            return value
        require(script == app.AI_CANCEL_JS, 'unexpected host browser operation')
        self.cancels += 1
        return await self.page.evaluate(script, args)


async def request(host, page, path):
    return await host._run_job(page, path, {}, 'synthetic-token',
                               'synthetic-bearer', path, None, 10)


async def drained(host, page):
    await empty_jobs(page.page)
    life = host.lifecycles['fixture']
    require(not life.activity.pending and not life.activity.activities,
            'host retained a completed request or RPC')
    require(page.starts == page.cancels == 1, 'request start or cleanup repeated')


async def deliveries(host, page, server, report):
    report['deliveries'] = []
    for fault in (None, 'before_poll', 'first_chunk', 'ack_next_chunk',
                  'final_chunk', 'close_receipt'):
        page.reset(fault)
        before = len(server.paths)
        value = await request(host, page, '/exact')
        check_body(value)
        require(set(value) == {'status', 'usage', 'body'}, 'protocol fields leaked')
        require(len(page.lossy.lost) == int(fault is not None), 'fault not exercised')
        require(server.paths[before:] == ['/exact'], 'HTTP request was repeated')
        await drained(host, page)
        report['deliveries'].append(dict(fault=fault, polls=page.polls,
            lost=page.lossy.lost, body_sha256=hashlib.sha256(value['body'].encode()).hexdigest()))
    report['checks'].append('host_delivery_survives_all_five_rpc_loss_points')


async def failures(host, page, server, report):
    page.reset()
    value = await request(host, page, '/oversize')
    require(value == dict(status=-1, usage=None,
        body='Error: response body limit exceeded', failure_phase='body', head_status=200),
        'host changed the terminal body failure')
    await asyncio.wait_for(server.disconnected['oversize'].wait(), 5)
    await drained(host, page)
    report['checks'].append('host_oversize_failure_discards_partial_output_and_joins')
    page.reset()
    waiter = asyncio.create_task(request(host, page, '/cancel'))
    try:
        await asyncio.wait_for(server.started['cancel'].wait(), 5)
    finally:
        waiter.cancel()
        result = await asyncio.gather(waiter, return_exceptions=True)
    require(isinstance(result[0], asyncio.CancelledError), 'held request escaped cancellation')
    await asyncio.wait_for(server.disconnected['cancel'].wait(), 5)
    await drained(host, page)
    report['checks'].append('host_cancellation_joins_reader_and_peer_close')


async def held_acknowledgement(host, page, report):
    page.reset(hold_final=True)
    waiter = asyncio.create_task(request(host, page, '/exact'))
    try:
        await asyncio.wait_for(page.entered.wait(), 10)
        waiter.cancel()
        result = await asyncio.gather(waiter, return_exceptions=True)
        require(isinstance(result[0], asyncio.CancelledError), 'ack waiter did not cancel')
        life = host.lifecycles['fixture']
        require(len(life.activity.pending) == 1, 'final RPC lost its account owner')
        require(not await host._close_account('fixture'), 'account closed over pending RPC')
        require(life.browser.is_connected(), 'browser closed before RPC settlement')
    finally:
        page.release.set()
        if not waiter.done():
            waiter.cancel()
        await asyncio.gather(waiter, return_exceptions=True)
        await asyncio.gather(*host.lifecycles['fixture'].activity.pending)
    await drained(host, page)
    report['checks'].append('cancelled_final_ack_retains_real_rpc_and_prevents_close')


async def account_cases(browser, server, report):
    context = await browser.new_context(ignore_https_errors=True)
    context.set_default_timeout(5000)
    real_page = await context.new_page()
    await real_page.goto(server.origin)
    require(app.AI_START_JS.count(scripts.API) == 1, 'expected one target origin')
    source = app.AI_START_JS.replace(scripts.API, server.origin)
    report['production_start_sha256'] = hashlib.sha256(app.AI_START_JS.encode()).hexdigest()
    report['executed_start_sha256'] = hashlib.sha256(source.encode()).hexdigest()
    page = HostPage(real_page, source)
    with tempfile.TemporaryDirectory() as directory, patch.object(app, 'ROOT', directory):
        host, life = app.Host(), app.AccountLifecycle()
        life.browser, life.phase = browser, 'resident'
        receipt = life.close_receipt = app.BrowserCloseReceipt(browser)
        account = app.Account(None, context, page, life)
        account.browser = browser
        host.accounts['fixture'], host.lifecycles['fixture'] = account, life
        try:
            await deliveries(host, page, server, report)
            await failures(host, page, server, report)
            await held_acknowledgement(host, page, report)
            require(server.paths == ['/exact'] * 6 + ['/oversize', '/cancel', '/exact']
                    and not server.errors, 'unexpected fixture request or handler failure')
        finally:
            await asyncio.wait_for(host.close(), 15)
        require(receipt.acknowledged and receipt.closed and life.phase == 'absent'
                and not browser.is_connected(), 'host did not confirm browser closure')
        report['checks'].append('host_browser_close_acknowledged')


async def exercise(browser_path, server, report):
    async with async_playwright() as playwright:
        browser = await playwright.firefox.launch(
            executable_path=str(browser_path), headless=True, timeout=15000,
            firefox_user_prefs={'dom.ipc.processCount': 1,
                                'dom.ipc.processCount.webIsolated': 1})
        try:
            report['browser_version'] = browser.version
            await account_cases(browser, server, report)
        finally:
            if browser.is_connected():
                await asyncio.wait_for(browser.close(), 5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser, exercise))
    result['scope'] = 'Local Host._run_job integration with synthetic credentials; no SDK or live account.'
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result['status'] == 'passed' else 1)
