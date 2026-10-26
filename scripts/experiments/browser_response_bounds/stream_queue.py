"""Measure a bounded response queue with production request ownership."""
import argparse
import asyncio
from functools import partial
import hashlib
import json
from pathlib import Path
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from probe import require, run, start
from memory import snapshot
from framerize_host.browser import scripts
from playwright.async_api import async_playwright

READER = (HERE / 'queue_reader.js').read_text()
POLL = (HERE / 'queue_poll.js').read_text()


def request_script(origin, queued):
    source = scripts.AI_START_JS
    require(source.count(scripts.API) == 1, 'expected one target origin')
    if queued:
        require(source.count(scripts._READ_RESPONSE_JS) == 1,
                'expected one production response reader')
        reader = ('(response, controller) => (' + READER
                  + f')(response, controller, job, {scripts.AI_RESPONSE_LIMIT})')
        source = source.replace(scripts._READ_RESPONSE_JS, reader)
    return source.replace(scripts.API, origin)


async def drain(page, job):
    parts, offset, calls, empty, peak = [], 0, 0, 0, 0
    while calls < 5000:
        value = await page.evaluate(POLL, job)
        calls += 1
        if value is None:
            empty += 1
            await asyncio.sleep(0.005)
            continue
        metrics = dict(calls=calls, empty_polls=empty, peak_queue_units=peak,
                       partial_units=offset)
        if value['status'] == -1:
            parts.clear()
            return value, metrics
        units = len(value['body'].encode('utf-16-le')) // 2
        require(units <= 131072 and value['peak_units'] <= 131072,
                'response queue exceeded its text bound')
        require(value['offset'] == offset and value['next'] == offset + units,
                'response queue lost or repeated content')
        offset = value['next']
        peak = max(peak, value['peak_units'])
        require(offset <= scripts.AI_RESPONSE_LIMIT, 'decoded response exceeded limit')
        parts.append(value['body'])
        if value['done'] is True:
            value['body'] = ''.join(parts)
            metrics.update(peak_queue_units=peak, partial_units=offset)
            return value, metrics
    raise RuntimeError('response queue exceeded its poll bound')


def check_body(value):
    body = value['body']
    require(value['status'] == 200 and value['usage'] == 'synthetic',
            'response status or usage changed')
    require(len(body) == scripts.AI_RESPONSE_LIMIT and body.count('a') == len(body),
            'response body differs from the complete fixture')


async def sample_while_running(stop, samples):
    started = time.perf_counter()
    while not stop.is_set():
        before = time.perf_counter()
        sample = await snapshot()
        sample['elapsed_ms'] = (time.perf_counter() - started) * 1000
        sample['sampling_ms'] = (time.perf_counter() - before) * 1000
        samples.append(sample)
        try:
            await asyncio.wait_for(stop.wait(), 0.05)
        except TimeoutError:
            pass


async def measure(page, source, mode, report):
    report['memory']['before_request'] = await snapshot()
    samples, stop = [], asyncio.Event()
    sampler = asyncio.create_task(sample_while_running(stop, samples))
    started = time.perf_counter()
    try:
        await start(page, source, '/exact')
        if mode == 'queued':
            value, metrics = await drain(page, '/exact')
        else:
            await page.wait_for_function("window.__aiJobs['/exact'].done")
            value = await page.evaluate(scripts.AI_POLL_JS, '/exact')
            metrics = dict(calls=1, empty_polls=0)
        metrics['request_to_body_ms'] = (time.perf_counter() - started) * 1000
    finally:
        stop.set()
        await asyncio.wait_for(sampler, 2)
        report['request_samples'] = samples
    report['memory']['after_request'] = await snapshot()
    check_body(value)
    digest = hashlib.sha256()
    for offset in range(0, len(value['body']), 65536):
        digest.update(value['body'][offset:offset + 65536].encode('ascii'))
    metrics.update(body_sha256=digest.hexdigest(), characters=len(value['body']))
    report['measurement'] = metrics
    report['checks'].append('complete_body_status_and_usage_intact')


async def check_failures(page, source, server, report):
    await start(page, source, '/exact')
    await page.wait_for_function("Boolean(window.__aiJobs['/exact']?.queue?.resume)")
    state = await page.evaluate("""() => {
      const job = window.__aiJobs['/exact'];
      return {done: job.done, units: job.queue.units, peak: job.queue.peakUnits};
    }""")
    require(not state['done'] and state['units'] == state['peak'] == 131072,
            'producer did not wait at the queue bound')
    require(await page.evaluate(scripts.AI_CANCEL_JS, '/exact') is True,
            'cancellation did not join a blocked producer')
    report['checks'].append('bounded_queue_backpressure_and_cancel_joined')
    await start(page, source, '/oversize')
    value, report['oversize_transfer'] = await drain(page, '/oversize')
    require(value['status'] == -1 and value['failure_phase'] == 'body'
            and value['head_status'] == 200
            and value['body'] == 'Error: response body limit exceeded',
            'oversize response did not retain its failure and observed head')
    await asyncio.wait_for(server.disconnected['oversize'].wait(), 5)
    report['checks'].append('oversize_refused_and_peer_closed')
    await start(page, source, '/cancel')
    await asyncio.wait_for(server.started['cancel'].wait(), 5)
    await page.wait_for_function("window.__aiJobs['/cancel']?.queue?.units > 0")
    require(await page.evaluate(scripts.AI_CANCEL_JS, '/cancel') is True,
            'held-body cancellation did not join')
    await asyncio.wait_for(server.disconnected['cancel'].wait(), 5)
    report['checks'].append('held_body_cancel_joined_and_peer_closed')


async def exercise_queue(browser_path, server, report, *, mode):
    source = request_script(server.origin, mode != 'full')
    report.update(mode=mode, browser_body_limit=scripts.AI_RESPONSE_LIMIT,
                  executed_start_sha256=hashlib.sha256(source.encode()).hexdigest())
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
            report['memory']['browser_idle'] = await snapshot()
            if mode == 'checks':
                await check_failures(page, source, server, report)
                expected_paths = ['/exact', '/oversize', '/cancel']
            else:
                await measure(page, source, mode, report)
                expected_paths = ['/exact']
            require(await page.evaluate('Object.keys(window.__aiJobs).length') == 0,
                    'browser retained a response job')
            require(server.paths == expected_paths and not server.errors,
                    'unexpected fixture request or handler failure')
            report['checks'].append('job_table_empty_and_only_expected_requests')
        finally:
            await asyncio.wait_for(browser.close(), 5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--mode', choices=('full', 'queued', 'checks'), required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser, partial(exercise_queue, mode=args.mode)))
    result['scope'] = 'Local bounded response queue; no SDK, account or upstream request.'
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result['status'] == 'passed' else 1)
