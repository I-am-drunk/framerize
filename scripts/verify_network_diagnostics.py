"""Check real browser error correlation using only local HTTP fixtures."""
import argparse
import asyncio
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import threading

from bench_browser_retention import memory_sample, require_isolation
from framerize_host.browser.network_failure import NetworkFailure


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def do_POST(self):
        wire = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        body = json.loads(wire)
        self.server.received[body['turnId']] = hashlib.sha256(wire).hexdigest()
        assert 'framerizeturnid' not in {key.lower() for key in self.headers}
        if body['fixture_status'] == 0:
            self.connection.shutdown(socket.SHUT_RDWR)
            self.connection.close()
            return
        self.send_response(body['fixture_status'])
        if body['fixture_status'] == 200:
            self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Content-Length', '0')
        self.end_headers()


    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'POST')
        self.send_header('Access-Control-Allow-Headers', 'content-type')
        self.end_headers()


FETCH = '''async ([url, body]) => {
  try {
    const response = await fetch(url, {method: 'POST', credentials: 'omit',
      headers: {'Content-Type': 'application/json'}, body});
    return {status: response.status, body: await response.text()};
  } catch (error) {
    return {status: -1, body: String(error)};
  }
}'''


async def run(args, server):
    from framerize_host.browser.driver_memory import install
    install(args.output.parent / (args.output.stem + '-runtime'))
    from playwright.async_api import async_playwright
    url = f'http://127.0.0.1:{server.server_port}/request'
    samples, seen, results = [], [], {}
    async with async_playwright() as driver:
        context = await driver.firefox.launch_persistent_context(
            str(args.output.parent / (args.output.stem + '-profile')),
            executable_path=str(args.browser), headless=True, timeout=60000,
            firefox_user_prefs={'browser.cache.memory.enable': False,
                'browser.cache.disk.enable': False, 'dom.ipc.processCount': 1,
                'framerize.captureRequestBodies': False,
                'framerize.captureResponseBodies': False,
                'framerize.captureRequestIdentity': True,
                'framerize.reportActualRedirects': True})
        page = context.pages[0]
        await page.goto(f'http://localhost:{server.server_port}/')
        def captured(request):
            if request.url == url and request.method == 'POST':
                identity = request._impl_obj._initializer.get('framerizeTurnId')
                seen.append((identity, request.post_data is None))
        page.on('request', captured)
        response_listeners = len(page._impl_obj.listeners('response'))
        async def one(turn, status):
            body = json.dumps({'turnId': turn, 'fixture_status': status,
                               'payload': 'x' * (128 * 1024)})
            observer = NetworkFailure(page, url, turn).watch()
            try:
                result = observer.explain(await page.evaluate(FETCH, [url, body]))
            finally:
                observer.close()
            assert server.received[turn] == hashlib.sha256(body.encode()).hexdigest()
            if status == 200:
                assert result['status'] == 200
            else:
                assert result['status'] == -1
                if args.baseline:
                    assert 'network_status' not in result
                elif status == 0:
                    assert result.get('network_failure'), result
                    assert 'network cause:' in result['body'], result
                else:
                    assert result.get('network_status') == status, result
                    assert f'HTTP {status}' in result['body'], result
            return {'browser_status': result['status'],
                    'observed_http_status': result.get('network_status'),
                    'network_failure': result.get('network_failure')}

        try:
            results['success'] = await one('success01', 200)
            results['reset'] = await one('reset01', 0)
            results['concurrent'] = await asyncio.gather(
                one('failure01', 504), one('failure02', 502))
            assert len(page._impl_obj.listeners('response')) == response_listeners
            for batch in range(args.batches):
                for index in range(16):
                    await one(f't{batch:02}{index:06}', 504)
                samples.append({'requests': (batch + 1) * 16, **memory_sample()})
                print(json.dumps({'requests': (batch + 1) * 16,
                    'pss_bytes': samples[-1]['pss_bytes']}), flush=True)
            count = args.batches * 16 + 4
            assert len(seen) == len(server.received) == count
            assert all(uncaptured for _, uncaptured in seen), 'Request bodies were retained'
            identities = [identity for identity, _ in seen]
            if args.baseline:
                assert all(identity is None for identity in identities)
            else:
                assert set(identities) == set(server.received)
            assert len(page._impl_obj.listeners('response')) == response_listeners
            assert len(page._impl_obj.listeners('requestfailed')) == 0
            width = max(1, len(samples) // 4)
            early = sum(s['pss_bytes'] for s in samples[width:2*width]) / width
            late = sum(s['pss_bytes'] for s in samples[-width:]) / width
            growth = late - early
            assert max(s['pss_bytes'] for s in samples) < 1024**3
            assert growth < 128 * 1024**2, ('Unexpected retained memory growth', growth)
            results.update(requests=count, payload_bytes=128 * 1024,
                bodies_uncaptured=True, identities_match=not args.baseline,
                listeners_released=True, pss_growth_bytes=growth,
                early_warm_mean_pss=early, late_mean_pss=late)
        finally:
            page.remove_listener('request', captured)
            await context.close()
    results['memory_samples'] = [{k: sample[k] for k in
        ('requests', 'pss_bytes', 'cgroup_bytes')} for sample in samples]
    args.output.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps({'result': str(args.output), 'checks': results}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--baseline', action='store_true')
    parser.add_argument('--batches', type=int, default=5)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output already exists')
    if not 4 <= args.batches <= 32:
        parser.error('batches must be between 4 and 32')
    require_isolation()
    os.environ.pop('FRAMERIZE_TOKEN_SOCK', None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    os.environ['FRAMERIZE_STATE'] = str(args.output.parent / (args.output.stem + '-state'))
    os.environ['FRAMERIZE_LOG_DIR'] = str(args.output.parent / (args.output.stem + '-logs'))
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.received = {}
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        asyncio.run(run(args, server))
    finally:
        server.shutdown()
        worker.join()
        server.server_close()
