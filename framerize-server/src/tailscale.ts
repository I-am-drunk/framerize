// Adapted from T3 Code packages/tailscale/src/tailscale.ts and cli/pair.ts at
// 5a906583fed3fa41dc806f2a19bd9d095537ed3e. MIT, T3 Tools Inc.
import { isIPv4 } from 'node:net';
import { runStep, ServerSetupError, type StepRunner } from './service-control.js';

const object = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

export function parseTailscaleStatus(raw: string) {
  const status: unknown = JSON.parse(raw);
  if (!object(status) || (status.Self !== undefined && !object(status.Self))) {
    throw new ServerSetupError('Tailscale returned an invalid status');
  }
  const self = object(status.Self) ? status.Self : {};
  const dns = typeof self.DNSName === 'string' ? self.DNSName.trim().replace(/\.$/, '') : '';
  const tailnetIpv4Addresses = Array.isArray(self.TailscaleIPs) ? self.TailscaleIPs.filter(
    (address): address is string => {
      if (typeof address !== 'string' || !isIPv4(address)) return false;
      const [first, second] = address.split('.').map(Number);
      return first === 100 && second! >= 64 && second! <= 127;
    }) : [];
  return { magicDnsName: dns || null, tailnetIpv4Addresses,
    httpsServeEnabled: object(self.CapMap) ? Object.hasOwn(self.CapMap, 'https') : null };
}

const command = () => process.platform === 'win32' ? 'tailscale.exe' : 'tailscale';
export const tailscaleServeOffArgs = (port: number): readonly string[] =>
  ['serve', `--https=${port}`, '--set-path=/', 'off'];

function existingTarget(raw: string, port: number): string | undefined {
  const config: unknown = JSON.parse(raw);
  if (!object(config)) throw new ServerSetupError('Invalid Tailscale Serve configuration');
  const matches: string[] = [];
  const scan = (value: unknown, nested = false): void => {
    if (!object(value)) return;
    const tcp = object(value.TCP) ? value.TCP[String(port)] : undefined;
    if (object(tcp) && tcp.HTTPS !== true) matches.push('another TCP service');
    if (object(value.Web)) for (const [host, web] of Object.entries(value.Web)) {
      if (!host.endsWith(`:${port}`) || !object(web) || !object(web.Handlers)) continue;
      for (const [mount, handler] of Object.entries(web.Handlers)) {
        matches.push(!nested && mount === '/' && object(handler) && typeof handler.Proxy === 'string'
          ? handler.Proxy : 'another web service');
      }
    }
    for (const key of ['Services', 'Foreground']) {
      if (object(value[key])) for (const child of Object.values(value[key])) scan(child, true);
    }
  };
  scan(config);
  if (matches.length > 1) throw new ServerSetupError(`Tailscale HTTPS port ${port} has other routes. Choose another port.`);
  return matches[0];
}

export async function prepareTailscale(localPort: number, servePort = 443, run: StepRunner = runStep,
  localHost = '127.0.0.1', previousTarget?: string) {
  if (![localPort, servePort].every(port => Number.isInteger(port) && port > 0 && port <= 65535)) {
    throw new ServerSetupError('Tailscale Serve requires fixed ports between 1 and 65535');
  }
  const status = await run(command(), ['status', '--json']);
  if (status.code !== 0) throw new ServerSetupError('Tailscale is not connected. Run tailscale up first.');
  const facts = parseTailscaleStatus(status.stdout);
  if (!facts.magicDnsName || !/^[A-Za-z0-9.-]+$/.test(facts.magicDnsName)) {
    throw new ServerSetupError('Tailscale has no usable MagicDNS name. Enable MagicDNS for this machine.');
  }
  if (facts.httpsServeEnabled === false) throw new ServerSetupError('Enable HTTPS certificates in the Tailscale admin console.');
  const host = localHost.includes(':') && !localHost.startsWith('[') ? `[${localHost}]` : localHost;
  const target = `http://${host}:${localPort}`;
  if (new URL(target).origin !== target) throw new ServerSetupError('Invalid local Tailscale target.');
  const configured = await run(command(), ['serve', 'status', '--json']);
  if (configured.code !== 0) throw new ServerSetupError('Could not inspect Tailscale Serve routes.');
  const existing = existingTarget(configured.stdout, servePort);
  if (existing !== undefined && existing.replace(/\/$/, '') !== target
    && existing.replace(/\/$/, '') !== previousTarget) {
    throw new ServerSetupError(`Tailscale HTTPS port ${servePort} already serves another application. Choose another port.`);
  }
  const origin = `https://${facts.magicDnsName}${servePort === 443 ? '' : `:${servePort}`}`;
  let attempted = false;
  let removed = false;
  let prior = existing;
  return { origin, configured: existing?.replace(/\/$/, '') === target, async apply() {
    const current = await run(command(), ['serve', 'status', '--json']);
    if (current.code !== 0) throw new ServerSetupError('Could not inspect Tailscale Serve routes.');
    const destination = existingTarget(current.stdout, servePort);
    if (destination?.replace(/\/$/, '') === target) return;
    if (destination !== undefined && destination.replace(/\/$/, '') !== previousTarget) {
      throw new ServerSetupError('The Tailscale route now belongs to another application.');
    }
    prior = destination;
    attempted = true;
    const result = await run(command(), ['serve', '--bg', `--https=${servePort}`, target]);
    if (result.code !== 0) throw new ServerSetupError('Could not configure Tailscale Serve.');
  }, async remove() {
    const current = await run(command(), ['serve', 'status', '--json']);
    if (current.code !== 0) throw new ServerSetupError('Could not inspect Tailscale Serve routes.');
    const destination = existingTarget(current.stdout, servePort);
    if (destination === undefined) return;
    if (destination.replace(/\/$/, '') !== target) throw new ServerSetupError('The Tailscale route now belongs to another application.');
    prior = destination;
    removed = true;
    if ((await run(command(), tailscaleServeOffArgs(servePort))).code !== 0) {
      throw new ServerSetupError('Could not remove the Server Tailscale route.');
    }
  }, async rollback() {
    if (removed && prior !== undefined) {
      const current = await run(command(), ['serve', 'status', '--json']);
      if (current.code !== 0) throw new ServerSetupError('Could not inspect Tailscale routes during rollback.');
      const destination = existingTarget(current.stdout, servePort);
      if (destination !== undefined && destination.replace(/\/$/, '') !== target) {
        throw new ServerSetupError('Another application now owns the Tailscale route; it was not changed.');
      }
      if ((await run(command(), ['serve', '--bg', `--https=${servePort}`, prior])).code !== 0) {
        throw new ServerSetupError('Could not restore the previous Tailscale route.');
      }
      return;
    }
    if (!attempted) return;
    const current = await run(command(), ['serve', 'status', '--json']);
    if (current.code !== 0) throw new ServerSetupError('Could not inspect Tailscale routes during rollback.');
    const destination = existingTarget(current.stdout, servePort)?.replace(/\/$/, '');
    if (destination === prior?.replace(/\/$/, '')) return;
    if (destination !== undefined && destination !== target) {
      if (prior !== undefined) throw new ServerSetupError('Another application now owns the Tailscale route; it was not changed.');
      return;
    }
    const args = prior === undefined ? tailscaleServeOffArgs(servePort)
      : ['serve', '--bg', `--https=${servePort}`, prior];
    if ((await run(command(), args)).code !== 0) {
      throw new ServerSetupError('Could not restore the previous Tailscale route.');
    }
  } };
}
