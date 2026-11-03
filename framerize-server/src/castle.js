import vm from 'node:vm';
import net from 'node:net';
import fs from 'node:fs';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ScriptModules } from '../castle/script_loader.mjs';
import { buildRealmGlobals } from '../castle/experiments/realm.mjs';
import { PUBLIC_KEY } from '../castle/pins.mjs';

const DEFAULT_BUDGET_MS = 8000;
const DEFAULT_TIMEOUT_MS = 9000;

function loadTrace(tracePath) {
  try {
    const parsed = JSON.parse(fs.readFileSync(tracePath, 'utf8'));
    if (Array.isArray(parsed.events)) return parsed.events;
  } catch {}
  return [];
}

function jitterTrace(trace) {
  const cx = 390, cy = 410;
  const ang = Math.random() * Math.PI * 2;
  const cos = Math.cos(ang), sin = Math.sin(ang);
  const dx = (Math.random() - 0.5) * 240;
  const dy = (Math.random() - 0.5) * 200;
  const rate = 0.85 + Math.random() * 0.5;
  return trace.map((ev) => {
    const e = { ...ev };
    if (typeof e.clientX === 'number' && typeof e.clientY === 'number') {
      const rx = (e.clientX - cx) * cos - (e.clientY - cy) * sin;
      const ry = (e.clientX - cx) * sin + (e.clientY - cy) * cos;
      e.clientX = Math.round(cx + rx + dx);
      e.clientY = Math.round(cy + ry + dy);
    }
    if (typeof e.dt === 'number') e.dt = e.dt / rate;
    return e;
  });
}

async function replayTrace(win, trace) {
  if (!win || typeof win.__dispatchCaptured !== 'function' || trace.length === 0) return 0;
  let fired = 0;
  const base = win.performance ? win.performance.now() : Date.now();
  const jittered = jitterTrace(trace);
  let prevDt = 0;
  for (const ev of jittered) {
    const { type, dt, ...props } = ev;
    const gap = (typeof dt === 'number' ? dt : prevDt) - prevDt;
    if (gap > 0) await new Promise((r) => setTimeout(r, Math.min(gap, 1000)));
    if (typeof dt === 'number') prevDt = dt;
    try {
      fired += win.__dispatchCaptured(type, {
        ...props, timeStamp: base + (typeof dt === 'number' ? dt : 0),
      }) || 0;
    } catch {}
  }
  return fired;
}
function encodeWireToken(encoderPath, fcltFrame, { timeoutMs = DEFAULT_TIMEOUT_MS, signal } = {}) {
  return new Promise((resolve, reject) => {
    const child = execFile(encoderPath, { maxBuffer: 4 * 1024 * 1024, timeout: timeoutMs, signal }, (err, stdout, stderr) => {
      if (err) return reject(new Error('encode-token: ' + (stderr || err.message)));
      const token = String(stdout).trim();
      if (!token) return reject(new Error('encode-token: empty'));
      resolve(token);
    });
    child.stdin.on('error', () => {});
    child.stdin.end(fcltFrame);
  });
}

export function createCastleMinter(options = {}) {
  const {
    encoderPath,
    tracePath = fileURLToPath(new URL('../castle/inputs/interaction-trace.json', import.meta.url)),
    budgetMs = DEFAULT_BUDGET_MS,
    timeoutMs = DEFAULT_TIMEOUT_MS,
  } = options;
  if (!encoderPath) throw new Error('castle encoderPath is required');
  const trace = loadTrace(tracePath);
  let busy = false;
  let mintCount = 0;
  let failCount = 0;

  async function mint({ cuid } = {}) {
    if (busy) {
      failCount++;
      throw new Error('castle minter busy');
    }
    busy = true;
    try {
      const globals = buildRealmGlobals({
        publicKey: PUBLIC_KEY,
        seedCuid: typeof cuid === 'string' && /^[A-Za-z0-9._~-]{1,256}$/.test(cuid) ? cuid : undefined,
      });
      const context = vm.createContext(globals);
      const modules = new ScriptModules(context);
      await modules.prepare();
      await modules.evaluate();
      const replayed = await replayTrace(globals, trace);
      let mintTimer;
      const collection = await Promise.race([
        modules.mint({ budget_ms: budgetMs }),
        new Promise((_, rej) => { mintTimer = setTimeout(() => rej(new Error('mint-timeout')), timeoutMs); }),
      ]).finally(() => clearTimeout(mintTimer));
      if (typeof collection !== 'string' || collection.length === 0) {
        throw new Error('collection-shape: ' + typeof collection);
      }
      const body = Buffer.from(collection, 'utf8');
      const header = Buffer.alloc(12);
      header.write('FCLT');
      header[4] = 1;
      header.writeUInt32BE(body.length, 8);
      const token = await encodeWireToken(encoderPath, Buffer.concat([header, body]), { timeoutMs });
      mintCount++;
      return {
        token,
        cuid: cuid || globals.window.localStorage.getItem('__cuid') || null,
        tokenLength: token.length,
        collectedLength: collection.length,
        replayed,
        traceEvents: trace.length,
        encoding: 'wire-token',
      };
    } catch (error) {
      failCount++;
      throw error;
    } finally {
      busy = false;
    }
  }

  function stats() {
    return { mints: mintCount, failures: failCount, busy, traceEvents: trace.length };
  }

  return { mint, stats };
}
