"""Check production response scripts in a privately networked browser."""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'runtime'),
               str(ROOT / 'scripts/experiments/shared_context_isolation')]
from fixture import ResponseServer
from memory import snapshot, visible_processes
from framerize_host.browser import scripts
from playwright.async_api import async_playwright


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


async def poll(page, job):
    return await page.evaluate('''job => {
      const value = (''' + scripts.AI_POLL_JS + ''')(job);
      if (value === null) return null;
      return {status: value.status, length: value.body.length,
        expected: /^a*$/.test(value.body), phase: value.failure_phase,
        head: value.head_status, error: value.status === -1 ? value.body : null};
    }''', job)


async def exercise(browser_path, server, report):
    source = scripts.AI_START_JS
    require(source.count(scripts.API) == 1, 'expected one target origin')
    local_start = source.replace(scripts.API, server.origin)
    report['production_script_sha256'] = hashlib.sha256(source.encode()).hexdigest()
    report['executed_script_sha256'] = hashlib.sha256(local_start.encode()).hexdigest()
    report['fixture_origin'] = server.origin
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
            await cases(page, local_start, server, report)
        finally:
            await asyncio.wait_for(browser.close(), 5)


async def start(page, source, path):
    return await page.evaluate(source, [path, {}, 'synthetic-token',
                                       'synthetic-bearer', path, None])


async def cases(page, source, server, report):
    await start(page, source, '/exact')
    await page.wait_for_function("window.__aiJobs['/exact'].done")
    report['memory']['exact_body_buffered'] = await snapshot()
    result = await poll(page, '/exact')
    require(result['status'] == 200 and result['expected']
            and result['length'] == scripts.AI_RESPONSE_LIMIT,
            'the exact byte limit did not arrive intact')
    report['checks'].append('exact_limit_body_intact')
    await start(page, source, '/oversize')
    await page.wait_for_function("window.__aiJobs['/oversize'].done")
    result = await poll(page, '/oversize')
    require(result['status'] == -1 and result['phase'] == 'body'
            and result['head'] == 200
            and result['error'] == 'Error: response body limit exceeded',
            'oversize response was not refused with its observed head')
    await asyncio.wait_for(server.disconnected['oversize'].wait(), 5)
    report['checks'].append('oversize_refused_and_connection_closed')
    await start(page, source, '/cancel')
    await asyncio.wait_for(server.started['cancel'].wait(), 5)
    require(await poll(page, '/cancel') is None, 'held response completed early')
    report['memory']['held_response'] = await snapshot()
    started = time.monotonic()
    require(await page.evaluate(scripts.AI_CANCEL_JS, '/cancel') is True,
            'cancellation did not join its job')
    await asyncio.wait_for(server.disconnected['cancel'].wait(), 5)
    report['cancel_and_peer_close_ms'] = (time.monotonic() - started) * 1000
    require(await poll(page, '/cancel') is None, 'cancelled job retained output')
    require(await page.evaluate('Object.keys(window.__aiJobs).length') == 0,
            'browser retained a request job')
    require(server.paths == ['/exact', '/oversize', '/cancel'], 'unexpected requests')
    require(not server.errors, 'fixture handler failed')
    report['checks'].append('cancellation_joined_and_peer_closed')


async def run(browser, exercise_operation=exercise):
    for kind in ('net', 'pid'):
        parent = os.environ.get('FIXTURE_PARENT_' + kind.upper())
        require(parent and os.readlink('/proc/self/ns/' + kind) != parent,
                'private network and PID namespaces are required')
    baseline = visible_processes()
    report = dict(checks=[], memory={}, status='failed', cleanup_joined=False)
    report['memory']['before_browser'] = await snapshot()
    failure = None
    with tempfile.TemporaryDirectory(prefix='response-bounds-') as directory:
        server = ResponseServer(scripts.AI_RESPONSE_LIMIT)
        await server.start(directory)
        try:
            await asyncio.wait_for(exercise_operation(browser, server, report), 45)
        except Exception as error:
            failure = type(error).__name__ + ': ' + str(error)
        finally:
            try:
                await server.close()
            except Exception as error:
                failure = 'server cleanup: ' + type(error).__name__
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
    report['cleanup_joined'] = remaining == {}
    if remaining != {}:
        failure = 'browser descendants remained or could not be checked'
    report['memory']['after_browser_close'] = await snapshot()
    report['status'] = 'failed' if failure else 'passed'
    report['failure'] = failure
    report['scope'] = 'Local browser response handling; no SDK, account or upstream request.'
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser))
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result['status'] == 'passed' else 1)
