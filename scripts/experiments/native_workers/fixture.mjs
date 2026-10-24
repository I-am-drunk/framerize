import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { DOM } from '../../offline_worker/pins.mjs';
import { installSharedWorkers } from '../../offline_worker/shared_workers.mjs';
import { installDedicatedWorkers } from './dedicated.mjs';
const { JSDOM, ResourceLoader } = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
export const script = `let count=0;self.onmessage=()=>postMessage([++count,navigator.userAgent,typeof navigator.userAgentData]);`;
const digest = text => createHash('sha256').update(text).digest('hex');
export function fixture(t, source = script, expected = source) {
  const dom = new JSDOM('', { url: 'https://framer.com/projects/test', runScripts: 'outside-only',
    resources: new ResourceLoader({ userAgent: 'Framerize/0.1' }) });
  const shared = installSharedWorkers(dom.window, digest('unused shared worker'));
  const bridge = installDedicatedWorkers(dom.window, [digest(expected)]);
  const url = dom.window.URL.createObjectURL(new dom.window.Blob([source], {type:'application/javascript'}));
  t.after(async () => { await bridge.close(); await shared.close(); dom.window.close(); });
  return { window: dom.window, bridge, url };
}
export function event(worker, type = 'message') {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('worker event timeout')), 2500);
    worker.addEventListener(type, value => { clearTimeout(timer); resolve(value); }, { once: true });
  });
}
export async function message(worker) {
  const pending = event(worker);
  worker.postMessage('');
  return (await pending).data;
}
