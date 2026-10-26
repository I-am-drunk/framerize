"""Compare full and chunked browser-to-host copies of one synthetic body."""
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

CHUNK = (HERE / 'transfer.js').read_text()


async def receive(page, mode):
    started = time.perf_counter()
    if mode == 'full':
        result = await page.evaluate(scripts.AI_POLL_JS, '/exact')
        require(isinstance(result, dict), 'completed body was missing')
        body, calls = result['body'], 1
    else:
        body, calls, result = await receive_chunks(page, int(mode))
    elapsed = (time.perf_counter() - started) * 1000
    require(result['status'] == 200 and result['usage'] == 'synthetic',
            'response status or usage changed during transfer')
    return body, dict(elapsed_ms=elapsed, calls=calls,
                      status=result['status'], usage=result['usage'])


async def receive_chunks(page, size):
    parts, offset, calls = [], 0, 0
    while calls < scripts.AI_RESPONSE_LIMIT // size + 1:
        result = await page.evaluate(CHUNK, ['/exact', offset, size])
        calls += 1
        part = result['body']
        require(isinstance(part, str) and 0 < len(part) <= size,
                'invalid ASCII fixture chunk')
        require(result['offset'] == offset and result['next'] == offset + len(part)
                and result['status'] == 200 and result['usage'] == 'synthetic',
                'chunk offset or response metadata changed')
        offset = result['next']
        require(offset <= scripts.AI_RESPONSE_LIMIT, 'chunk exceeded fixture limit')
        parts.append(part)
        if result['done'] is True:
            return ''.join(parts), calls, result
    raise RuntimeError('chunk transfer did not finish within the call bound')


async def exercise_transfer(browser_path, server, report, *, mode):
    source = scripts.AI_START_JS
    require(source.count(scripts.API) == 1, 'expected one target origin')
    local_start = source.replace(scripts.API, server.origin)
    report.update(mode=mode, fixture_origin=server.origin,
                  body_bytes=scripts.AI_RESPONSE_LIMIT)
    report['script_sha256'] = {
        'start': hashlib.sha256(source.encode()).hexdigest(),
        'executed_start': hashlib.sha256(local_start.encode()).hexdigest(),
        'poll': hashlib.sha256(scripts.AI_POLL_JS.encode()).hexdigest(),
        'chunk': hashlib.sha256(CHUNK.encode()).hexdigest()}
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
            await start(page, local_start, '/exact')
            await page.wait_for_function("window.__aiJobs['/exact'].done")
            report['memory']['before_transfer'] = await snapshot()
            body, report['transfer'] = await receive(page, mode)
            report['memory']['after_transfer'] = await snapshot()
            await verify_transfer(page, body, server, report)
        finally:
            await asyncio.wait_for(browser.close(), 5)


async def verify_transfer(page, body, server, report):
    require(len(body) == scripts.AI_RESPONSE_LIMIT and body.count('a') == len(body),
            'transferred body differs from the complete ASCII fixture')
    digest = hashlib.sha256()
    for offset in range(0, len(body), 65536):
        digest.update(body[offset:offset + 65536].encode('ascii'))
    report['transfer'].update(body_sha256=digest.hexdigest(), characters=len(body))
    require(await page.evaluate('Object.keys(window.__aiJobs).length') == 0,
            'browser retained a request job after transfer')
    require(server.paths == ['/exact'], 'unexpected fixture request')
    require(not server.errors, 'fixture handler failed')
    report['checks'].extend(['complete_body_identical', 'status_and_usage_intact',
                             'browser_job_removed', 'only_exact_fixture_requested'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--mode', choices=('full', '32768', '131072'), required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser, partial(exercise_transfer, mode=args.mode)))
    result['scope'] = 'Local completed-body transfer; no SDK, account or upstream request.'
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result['status'] == 'passed' else 1)
