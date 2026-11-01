"""Measure resident contexts during identical, account-free local HTTP load."""
import argparse
import asyncio
from contextlib import nullcontext
from http.server import ThreadingHTTPServer
import json
import os
from pathlib import Path
import threading

from bench_browser_retention import Handler, LOAD, memory_sample, require_isolation

ACTIVE_STREAM = """async () => {
  const state = {bytes: 0, done: false, error: null};
  window.activeFixture = state;
  const response = await fetch('/active-stream');
  if (!response.ok) throw new Error('active fixture fetch failed');
  const reader = response.body.getReader();
  state.task = (async () => {
    try {
      while (true) {
        const {value, done} = await reader.read();
        if (done) {state.done = true; return;}
        state.bytes += value.byteLength;
      }
    } catch (error) {state.error = String(error);}
    finally {reader.releaseLock();}
  })();
}"""


async def run(args, url):
    from framerize_host import app
    from playwright.async_api import async_playwright
    if args.fixed:
        from framerize_host.accounts.device_use import device_use
        from framerize_host.browser.driver_memory import install
        install(args.output.parent / (args.output.stem + '-runtime'))
    # No website, session, SDK or token is involved in this lifecycle fixture.
    app.LOGIN_URL, app.WARM_JS = url, 'true'
    samples = []
    host = app.Host()
    async with async_playwright() as driver:
        browser = await driver.firefox.launch(executable_path=str(args.browser),
            headless=True, firefox_user_prefs={
                'dom.ipc.processCount': 1, 'dom.ipc.processCount.webIsolated': 1,
                'browser.cache.memory.enable': False, 'browser.cache.disk.enable': False,
                'framerize.captureResponseBodies': False,
                'framerize.captureRequestBodies': False,
                'framerize.reportActualRedirects': True,
                'framerize.reclaimClosedContexts': args.fixed,
                'framerize.memoryReportPath': str(args.output.with_suffix(''))
                    if args.memory_report else ''})
        context = await browser.new_context()
        page = await context.new_page()
        life = app.AccountLifecycle()
        life.browser, life.phase = browser, 'resident'
        acc = app.Account(None, context, page, life)
        acc.browser = browser
        life.close_receipt = app.BrowserCloseReceipt(browser)
        host.accounts['fixture'], host.lifecycles['fixture'] = acc, life
        acc.devices[:] = ['primary'] + [f'device-{i}' for i in range(args.devices)]
        acc.primary_dev = 'primary'

        def owner(name):
            return device_use(host, 'fixture', acc, name) if args.fixed else nullcontext()

        def record(requests):
            sample = dict(requests=requests, contexts=len(browser.contexts),
                pages=sum(len(c.pages) for c in browser.contexts), **memory_sample())
            samples.append(sample)
            print(json.dumps({k:v for k,v in sample.items() if k != 'processes'}), flush=True)

        try:
            async with host._use_account('fixture'), owner('primary'):
                primary = await host._dev_page(acc, 'fixture', 'primary')
                await primary.page.evaluate(ACTIVE_STREAM)
                previous_bytes = 0
                record(0)
                for cycle in range(args.rounds):
                    for index, name in enumerate(acc.devices[1:]):
                        async with owner(name):
                            device = await host._dev_page(acc, 'fixture', name)
                            await host._rpc('fixture', device.page.evaluate, LOAD,
                                {'batch':cycle, 'page':index, 'count':8, 'consoleObjects':False})
                        trimming = getattr(acc, 'trim_task', None)
                        if trimming is not None:
                            await asyncio.wait_for(asyncio.shield(trimming), 20)
                        assert (await primary.page.evaluate('document.body.textContent')).strip() == 'Local'
                    await host._monitor_once()
                    if args.gc_between_rounds:
                        await primary.page.request_gc()
                    await asyncio.sleep(0.25)
                    record((cycle+1)*args.devices*8)
                    activity = await primary.page.evaluate('''() => ({
                        bytes: activeFixture.bytes, done: activeFixture.done,
                        error: activeFixture.error})''')
                    samples[-1]['active_stream_state'] = activity
                    assert not activity['done'] and activity['error'] is None, activity
                    assert activity['bytes'] > previous_bytes, activity
                    previous_bytes = activity['bytes']
                    samples[-1]['active_stream_bytes'] = previous_bytes
                if args.memory_report:
                    # Only the separate diagnostic browser implements this.
                    # It records allocations before and after collection.
                    await primary.page.request_gc()
        finally:
            await host.close()
            args.output.write_text(json.dumps({'fixed':args.fixed,
                'devices':args.devices, 'rounds':args.rounds,
                'active_stream': True,
                'gc_between_rounds':args.gc_between_rounds, 'samples':samples}, indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--fixed', action='store_true')
    parser.add_argument('--devices', type=int, default=8)
    parser.add_argument('--rounds', type=int, default=8)
    parser.add_argument('--gc-between-rounds', action='store_true')
    parser.add_argument('--memory-report', action='store_true')
    args = parser.parse_args()
    if args.output.exists() or not 1 <= args.devices <= 12 or not 1 <= args.rounds <= 16:
        parser.error('output must be new and workload within its fixed bounds')
    require_isolation()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    os.environ.pop('FRAMERIZE_TOKEN_SOCK', None)
    os.environ['FRAMERIZE_STATE'] = str(args.output.parent / (args.output.stem+'-state'))
    os.environ['FRAMERIZE_LOG_DIR'] = str(args.output.parent / (args.output.stem+'-logs'))
    Path(os.environ['FRAMERIZE_LOG_DIR']).mkdir(exist_ok=True)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        asyncio.run(run(args, 'http://127.0.0.1:'+str(server.server_port)))
    finally:
        server.shutdown()
        thread.join()
        server.server_close()
