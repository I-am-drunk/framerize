import fs from 'node:fs';
import { SavedDevice } from './device.mjs';

const INFERENCE_PATHS = Object.freeze([
  '/ai/v3/responses/', '/ai/v3/messages/', '/ai/v3/chat/',
  '/ai/v3/openai/responses/', '/ai/v3/anthropic/messages/',
]);

export function validateInput(input, residentPage = false) {
  const fields = input?.version === 2 ? 'budget_ms,device,target,version' : 'budget_ms,target,version';
  if (!input || Object.keys(input).sort().join(',') !== fields
      || ![1, 2].includes(input.version) || !Number.isInteger(input.budget_ms)
      || input.budget_ms < 1 || input.budget_ms > 16000) throw new Error('input');
  const target = input.target;
  if (!target || Object.keys(target).sort().join(',') !== 'path,project_id,url'
      || typeof target.project_id !== 'string' || !/^[A-Za-z0-9_-]{1,256}$/.test(target.project_id)
      || typeof target.url !== 'string' || target.url.length > 4096
      || !INFERENCE_PATHS.includes(target.path)) throw new Error('target');
  let url;
  try { url = new URL(target.url); } catch { throw new Error('target'); }
  if (url.origin !== 'https://framer.com' || url.username || url.password || url.port)
    throw new Error('target');
  const project = url.pathname.replace(/\/+$/, '').replace(/^\/projects\//, '');
  const linked = /^(?:(?:[A-Za-z0-9]+-)*[A-Za-z0-9]+--)?([A-Za-z0-9]{20})-[A-Za-z0-9]+$/.exec(project);
  const matches = project === target.project_id || project.endsWith('--' + target.project_id)
    || linked?.[1] === target.project_id;
  // Only the explicit resident V2 entry may use these captured app documents.
  // Its project authority belongs to the retained HTTP-200 provenance owner.
  const residentDocument = residentPage === true && input.version === 2
    && (url.pathname === '/login' || url.pathname === '/projects/');
  if (!residentDocument && (!url.pathname.startsWith('/projects/') || project.includes('/')
      || !matches))
    throw new Error('target');
  const device = input.version === 2 ? new SavedDevice(input.device) : undefined;
  return Object.freeze({ version: input.version, budget_ms: input.budget_ms,
    target: Object.freeze({ ...target }), ...(device ? { device } : {}) });
}

export class OpaqueToken {
  #value;
  constructor(value) { this.#value = value; }
  take() {
    if (this.#value === undefined) throw new Error('token-consumed');
    const value = this.#value; this.#value = undefined; return value;
  }
  toJSON() { throw new Error('private-token'); }
}

export function verifyDescriptors() {
  for (const fd of [0, 1, 2])
    if (fs.realpathSync('/proc/self/fd/' + fd) !== '/dev/null') throw new Error('stdio');
  const seen = new Set();
  for (const [fd, mode] of [[3, 0], [4, 1], [5, 1]]) {
    const stat = fs.fstatSync(fd);
    const match = /^flags:\s+([0-7]+)$/m.exec(fs.readFileSync('/proc/self/fdinfo/' + fd, 'utf8'));
    const identity = stat.dev + ':' + stat.ino;
    if (!stat.isFIFO() || !match || (parseInt(match[1], 8) & 3) !== mode || seen.has(identity))
      throw new Error('descriptor');
    seen.add(identity);
  }
}

export function readInput() {
  const chunks = []; let length = 0;
  try {
    for (;;) {
      const chunk = Buffer.alloc(1024);
      const count = fs.readSync(3, chunk);
      if (count === 0) break;
      length += count;
      if (length > 8192) throw new Error('input-size');
      chunks.push(chunk.subarray(0, count));
    }
    const json = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks));
    // The minter validates once and retains the private saved-device handle.
    return JSON.parse(json);
  } catch { throw new Error('input'); }
  finally { fs.closeSync(3); for (const chunk of chunks) chunk.fill(0); }
}
