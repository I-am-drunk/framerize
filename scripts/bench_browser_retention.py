"""Compare browser retention using identical local HTTP requests, without accounts."""
import argparse
import asyncio
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import threading
import time

PAYLOAD = b'x' * (256 * 1024)
MEMORY_LIMIT = 2 * 1024**3


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/active-stream':
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Transfer-Encoding', 'chunked')
            self.end_headers()
            try:
                # Outlive the 15-minute measurement deadline on slow builders.
                for _ in range(18000):
                    self.wfile.write(b'1\r\nx\r\n')
                    self.wfile.flush()
                    time.sleep(0.1)
                self.wfile.write(b'0\r\n\r\n')
            except (BrokenPipeError, ConnectionResetError):
                pass
            return
        self.send_response(200)
        self.send_header('Content-Type', 'text/html')
        self.send_header('Content-Length', '32')
        self.end_headers()
        self.wfile.write(b'<html><body>Local</body></html>\n')

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        if body != b'q' * (128 * 1024):
            self.send_error(400, 'upload changed')
            return
        self.send_response(200)
        self.send_header('Cache-Control', 'no-store')
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(len(PAYLOAD)))
        self.end_headers()
        prefix = self.path.encode()[:32].ljust(32, b' ')
        self.wfile.write(prefix + PAYLOAD[32:])

    def log_message(self, *_):
        pass


def memory_sample():
    group = Path('/proc/self/cgroup').read_text().strip().split('0::')[-1]
    cgroup = Path('/sys/fs/cgroup') / group.lstrip('/')
    pids = (cgroup / 'cgroup.procs').read_text().split()
    pss = rss = 0
    processes = []
    for pid in pids:
        try:
            values = dict(line.split(':', 1) for line in
                          Path('/proc', pid, 'smaps_rollup').read_text().splitlines()[1:])
            resident = int(values['Rss'].split()[0]) * 1024
            proportional = int(values['Pss'].split()[0]) * 1024
            rss += resident
            pss += proportional
            processes.append({'name': Path('/proc', pid, 'comm').read_text().strip(),
                              'rss_bytes': resident, 'pss_bytes': proportional})
        except (OSError, KeyError, ValueError):
            continue
    return {'pss_bytes': pss, 'rss_bytes': rss, 'processes': processes,
            'cgroup_bytes': int((cgroup / 'memory.current').read_text())}


async def run(args, url):
    if args.patched_driver:
        from framerize_host.browser.driver_memory import install
        install(args.output.parent / (args.output.stem + '-runtime'))
    from playwright.async_api import async_playwright
    samples = []
    def record(requests):
        sample = {'requests': requests, **memory_sample()}
        samples.append(sample)
        print(json.dumps({k: v for k, v in sample.items() if k != 'processes'}), flush=True)
    async with async_playwright() as driver:
        context = await driver.firefox.launch_persistent_context(
            str(args.output.parent / (args.output.stem + '-profile')),
            executable_path=str(args.browser), headless=True, timeout=60000,
            firefox_user_prefs={'browser.cache.memory.enable': False,
                'browser.cache.disk.enable': False, 'dom.ipc.processCount': 1,
                'framerize.captureResponseBodies': False,
                'framerize.captureRequestBodies': False,
                'framerize.reportActualRedirects': True})
        try:
            pages = [context.pages[0]]
            for _ in range(args.pages - 1):
                pages.append(await context.new_page())
            for page in pages:
                await page.goto(url)
            record(0)
            for batch in range(args.batches):
                for index, page in enumerate(pages):
                    await asyncio.wait_for(page.evaluate(LOAD, {'batch': batch, 'page': index,
                                              'count': args.requests,
                                              'consoleObjects': args.console_objects}), 120)
                await asyncio.sleep(0.25)
                record((batch+1)*args.requests*args.pages)
        finally:
            await context.close()
    args.output.write_text(json.dumps({'samples': samples, 'pages': args.pages,
        'payload_bytes': len(PAYLOAD), 'console_objects': args.console_objects,
        'patched_driver': args.patched_driver}, indent=2)+'\n')
    print(json.dumps({'output': str(args.output), 'samples': [
        {k:v for k,v in s.items() if k != 'processes'} for s in samples]}), flush=True)


LOAD = """async ({batch, page, count, consoleObjects}) => {
  const body = 'q'.repeat(128 * 1024);
  for (let i = 0; i < count; i++) {
    const path = '/blob?batch=' + batch + '&page=' + page + '&n=' + i;
    const response = await fetch(path, {method: 'POST', body});
    if (consoleObjects) {
      const payload = new Uint8Array(128 * 1024);
      payload.fill(100);
      console.log({path, payload});
    }
    if (!response.ok) throw new Error('local upload failed');
    const text = await response.text();
    if (text.length !== 256 * 1024 || text[32] !== 'x' ||
        text.slice(0, 32) !== path.slice(0, 32).padEnd(32))
      throw new Error('local response changed');
  }
}"""


def require_isolation():
    group = Path('/proc/self/cgroup').read_text().strip().split('0::')[-1]
    cgroup = Path('/sys/fs/cgroup') / group.lstrip('/')
    limit = (cgroup / 'memory.max').read_text().strip()
    isolated_container = (Path('/.dockerenv').exists()
        and os.environ.get('FRAMERIZE_ISOLATED_RETENTION') == '1')
    if (('framerize-retention-' not in group and not isolated_container)
            or limit == 'max' or int(limit) > MEMORY_LIMIT):
        raise RuntimeError('run in a separate framerize-retention- unit capped at 2 GiB or less')
    mem = {k: int(v.split()[0])*1024 for k,v in
           (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())}
    if mem['MemAvailable'] < mem['MemTotal']//10 + int(limit):
        raise RuntimeError('insufficient host headroom for the isolated browser measurement')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--pages', type=int, default=2)
    parser.add_argument('--batches', type=int, default=8)
    parser.add_argument('--requests', type=int, default=64)
    parser.add_argument('--patched-driver', action='store_true')
    parser.add_argument('--console-objects', action='store_true')
    args = parser.parse_args()
    if not 1 <= args.pages <= 2 or not 1 <= args.batches <= 16 or not 1 <= args.requests <= 128:
        parser.error('measurement exceeds its bounded workload')
    if args.output.exists():
        parser.error('output already exists')
    require_isolation()
    os.environ.pop('FRAMERIZE_TOKEN_SOCK', None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever)
    worker.start()
    try:
        asyncio.run(run(args, 'http://127.0.0.1:' + str(server.server_port)))
    finally:
        server.shutdown()
        worker.join()
        server.server_close()
