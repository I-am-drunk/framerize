"""Exercise acknowledged response delivery with deliberately lost RPC replies."""
import argparse
import asyncio
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from probe import require, run, start
from queue_ack import CAPACITY, POLL, drain
from stream_queue import check_body, request_script
from framerize_host.browser import scripts
from playwright.async_api import async_playwright


class LossyPage:
    def __init__(self, page, fault):
        self.page, self.fault, self.lost = page, fault, []

    async def evaluate(self, script, args):
        if self.fault == 'before_poll' and not self.lost:
            self.lost.append(dict(kind='before', acknowledged=args[1]))
            raise asyncio.TimeoutError('fixture timeout before poll execution')
        value = await self.page.evaluate(script, args)
        kind, sequence = value['kind'], value.get('sequence')
        lose = ((self.fault == 'first_chunk' and kind == 'chunk' and sequence == 0)
                or (self.fault == 'ack_next_chunk' and kind == 'chunk' and
                    args[1] >= 0 and not value['done'])
                or (self.fault == 'final_chunk' and kind == 'chunk' and value['done'])
                or (self.fault == 'close_receipt' and kind == 'closed'))
        if lose and not self.lost:
            self.lost.append(dict(kind=kind, sequence=sequence, acknowledged=args[1]))
            raise asyncio.TimeoutError('fixture timeout after poll execution')
        return value


async def empty_jobs(page):
    require(await page.evaluate('Object.keys(window.__aiJobs).length') == 0,
            'browser retained a response job')


async def successful_deliveries(page, source, server, report):
    for fault in (None, 'before_poll', 'first_chunk', 'ack_next_chunk',
                  'final_chunk', 'close_receipt'):
        before = len(server.paths)
        await start(page, source, '/exact')
        lossy = LossyPage(page, fault)
        try:
            value, metrics = await drain(lossy, '/exact')
            check_body(value)
            require(len(lossy.lost) == int(fault is not None), 'fault was not exercised')
            require(metrics['retries'] == len(lossy.lost), 'unexpected delivery retries')
            require(server.paths[before:] == ['/exact'], 'retry repeated the HTTP request')
            await empty_jobs(page)
            report['deliveries'].append(dict(fault=fault, metrics=metrics, lost=lossy.lost))
        finally:
            await asyncio.wait_for(page.evaluate(scripts.AI_CANCEL_JS, '/exact'), 5)
    report['checks'].append('complete_body_survives_all_five_rpc_loss_points')


async def full_queue_cancel(page, source, report):
    await start(page, source, '/exact')
    try:
        await page.wait_for_function("Boolean(window.__aiJobs['/exact']?.queue?.resume)")
        frame = await page.evaluate(POLL, ['/exact', -1])
        require(frame['kind'] == 'chunk' and len(frame['body']) == CAPACITY,
                'expected a full unacknowledged chunk')
        require(await page.evaluate(POLL, ['/exact', -1]) == frame,
                'retry changed an unacknowledged chunk')
        state = await page.evaluate("""() => {
          const job = window.__aiJobs['/exact'], q = job.queue;
          return {units: q.units, waiting: Boolean(q.resume), done: job.done,
            retained: job.delivery.pending.units + q.parts.join('').length};
        }""")
        require(state == dict(units=CAPACITY, waiting=True, done=False, retained=CAPACITY),
                'unacknowledged text did not retain queue capacity')
    finally:
        require(await asyncio.wait_for(page.evaluate(scripts.AI_CANCEL_JS, '/exact'), 5) is True,
                'full-queue cancellation did not join')
    await empty_jobs(page)
    report['checks'].append('unacknowledged_frame_retains_capacity_and_cancel_joins')


async def failed_deliveries(page, source, server, report):
    await start(page, source, '/oversize')
    try:
        value, _ = await drain(page, '/oversize')
        require(value['status'] == -1 and value['failure_phase'] == 'body'
                and value['head_status'] == 200
                and value['body'] == 'Error: response body limit exceeded',
                'oversize delivery lost its failure or observed head')
        await asyncio.wait_for(server.disconnected['oversize'].wait(), 5)
        await empty_jobs(page)
    finally:
        await asyncio.wait_for(page.evaluate(scripts.AI_CANCEL_JS, '/oversize'), 5)
    report['checks'].append('oversize_failure_acknowledged_and_peer_closed')
    await start(page, source, '/cancel')
    try:
        await asyncio.wait_for(server.started['cancel'].wait(), 5)
        await page.wait_for_function("window.__aiJobs['/cancel']?.queue?.units > 0")
        pending = await page.evaluate(POLL, ['/cancel', -1])
        require(pending['kind'] == 'chunk' and not pending['done'],
                'held network response completed early')
        await page.evaluate("window.__aiJobs['/cancel'].controller.abort()")
        value, _ = await drain(page, '/cancel')
        require(value['status'] == -1 and value['failure_phase'] == 'body'
                and value['head_status'] == 200, 'abort published success')
        await asyncio.wait_for(server.disconnected['cancel'].wait(), 5)
        await empty_jobs(page)
    finally:
        await asyncio.wait_for(page.evaluate(scripts.AI_CANCEL_JS, '/cancel'), 5)
    report['checks'].append('abort_with_pending_frame_joins_failure_and_peer_close')


async def exercise(browser_path, server, report):
    source = request_script(server.origin, True)
    report['deliveries'] = []
    async with async_playwright() as playwright:
        browser = await playwright.firefox.launch(
            executable_path=str(browser_path), headless=True, timeout=15000,
            firefox_user_prefs={'dom.ipc.processCount': 1,
                                'dom.ipc.processCount.webIsolated': 1})
        try:
            report['browser_version'] = browser.version
            context = await browser.new_context(ignore_https_errors=True)
            context.set_default_timeout(5000)
            page = await context.new_page()
            await page.goto(server.origin)
            await successful_deliveries(page, source, server, report)
            await full_queue_cancel(page, source, report)
            await failed_deliveries(page, source, server, report)
            require(server.paths == ['/exact'] * 7 + ['/oversize', '/cancel']
                    and not server.errors, 'unexpected fixture request or handler failure')
        finally:
            await asyncio.wait_for(browser.close(), 5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser, exercise))
    result['scope'] = 'Local acknowledgement and RPC-loss fixture; no SDK or real account.'
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result['status'] == 'passed' else 1)
