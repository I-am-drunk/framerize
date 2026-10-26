"""Time the same local queue and full-body paths without concurrent PSS reads."""
import argparse
import asyncio
from functools import partial
import json
from pathlib import Path
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from stream_queue import request_script, drain, check_body
from probe import require, run, start
from memory import snapshot
from framerize_host.browser import scripts
from playwright.async_api import async_playwright


async def exercise_timing(browser_path, server, report, *, mode):
    source = request_script(server.origin, mode == 'queued')
    report.update(mode=mode, request_pss_sampler=False)
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
            report['memory']['before_request'] = await snapshot()
            started = time.perf_counter()
            await start(page, source, '/exact')
            if mode == 'queued':
                value, metrics = await drain(page, '/exact')
            else:
                await page.wait_for_function("window.__aiJobs['/exact'].done")
                value = await page.evaluate(scripts.AI_POLL_JS, '/exact')
                metrics = dict(calls=1, empty_polls=0)
            metrics['request_to_body_ms'] = (time.perf_counter() - started) * 1000
            report['memory']['after_request'] = await snapshot()
            check_body(value)
            report['measurement'] = metrics
            report['checks'].append('complete_body_status_and_usage_intact')
            require(await page.evaluate('Object.keys(window.__aiJobs).length') == 0,
                    'browser retained a response job')
            require(server.paths == ['/exact'] and not server.errors,
                    'unexpected fixture request or handler failure')
            report['checks'].append('job_table_empty_and_only_expected_requests')
        finally:
            await asyncio.wait_for(browser.close(), 5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--mode', choices=('full', 'queued'), required=True)
    args = parser.parse_args()
    result = asyncio.run(run(args.browser, partial(exercise_timing, mode=args.mode)))
    result['scope'] = 'Local timing without request PSS sampling; no SDK or upstream.'
    print(json.dumps(result, sort_keys=True), flush=True)
    sys.exit(0 if result['status'] == 'passed' else 1)
