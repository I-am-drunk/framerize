import assert from 'node:assert/strict';
import dns, { type LookupAddress, type LookupAllOptions } from 'node:dns';
import { setImmediate as nextTurn } from 'node:timers/promises';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import { backend, deferred, token } from './http-fixtures.js';

async function settlesWithin(promise: Promise<unknown>, milliseconds: number): Promise<boolean> {
  let timer: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([promise.then(() => true), new Promise<boolean>(resolve => {
      timer = setTimeout(() => resolve(false), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

for (const lookupFails of [false, true]) {
  test(`close cancels pending listen before DNS ${lookupFails ? 'fails' : 'succeeds'}`,
    { timeout: 5000 }, async context => {
    const entered = deferred<void>();
    const original = dns.lookup;
    let releaseDns = (): void => {};
    let closes = 0, listeningEvents = 0;
    context.mock.method(dns, 'lookup', (host: string, options: LookupAllOptions,
      callback: (error: NodeJS.ErrnoException | null, addresses: LookupAddress[]) => void) => {
      if (host !== 'pending-listen.invalid') return original(host, options, callback);
      let released = false;
      releaseDns = () => {
        if (released) return;
        released = true;
        callback(lookupFails ? Object.assign(new Error('synthetic DNS failure'), { code: 'ENOTFOUND' }) : null,
          lookupFails ? [] : [{ address: '127.0.0.1', family: 4 }]);
      };
      entered.resolve();
    });
    const instance = createInstance({ host: 'pending-listen.invalid', port: 0, token,
      shutdownTimeoutMs: 50, backend: backend({ close() { closes += 1; } }) });
    instance.server.on('listening', () => { listeningEvents += 1; });
    const listening = instance.listen().then(address => ({ address, error: undefined }),
      (error: unknown) => ({ address: undefined, error }));
    context.after(async () => { releaseDns(); await listening; await instance.close(); });
    await entered.promise;
    const closing = instance.close();
    assert.equal(instance.close(), closing);
    const closedBeforeDns = await settlesWithin(closing, 1000);
    const backendClosedBeforeDns = closes;
    releaseDns();
    const result = await listening;
    await closing;
    await nextTurn();
    assert.equal(closedBeforeDns, true, 'shutdown must not wait for DNS');
    assert.equal(backendClosedBeforeDns, 1, 'backend closes while DNS is pending');
    assert.equal(result.address, undefined);
    assert.ok(result.error instanceof Error);
    assert.equal(result.error.message, 'Server is closing');
    assert.equal(listeningEvents, 0, 'late DNS must not open a listener');
    assert.equal(instance.server.address(), null);
    assert.equal(closes, 1);
  });
}
