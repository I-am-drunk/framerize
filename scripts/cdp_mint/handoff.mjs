// One-command handoff for the runtime owner. Mints a genuine Castle token with
// the CDP headless engine (the ~200 MB RAM win), then assembles the exact
// live request file the tls_sender reads. Does NOT fire the request: the
// authorized live shot is a separate, deliberate step.
//
// usage:
//   node scripts/cdp_mint/handoff.mjs <projectId> <bearerToken> <cookieHeader> [tokenFile]
//
// It runs engine.mjs as a child so the mint gets its own port/profile, reads
// the minted token from the file the engine writes, and writes
// scripts/tls_sender/live_request.json with the CDP token in place.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const [projectId, bearer, cookie, tokenFileArg] = process.argv.slice(2);
if (!projectId || !bearer || !cookie) {
  console.error('usage: handoff.mjs <projectId> <bearerToken> <cookieHeader> [tokenFile]');
  process.exit(2);
}
if (!/^https?:\/\//.test('https://framer.com/projects/' + projectId)) {
  console.error('bad projectId');
  process.exit(2);
}

const here = path.dirname(new URL(import.meta.url).pathname);
const engine = path.join(here, 'engine.mjs');
const editorUrl = 'https://framer.com/projects/' + projectId;
const runId = 'handoff-' + process.pid;
const tokenFile = tokenFileArg || ('/tmp/cdp-wire-token-' + runId + '.txt');

function mint() {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [engine, editorUrl], {
      env: { ...process.env, CDP_MINT_RUN: runId, CDP_MINT_TOKEN_OUT: tokenFile },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let out = '';
    child.stdout.on('data', d => { out += d; });
    child.stderr.on('data', () => {});
    const timer = setTimeout(() => { child.kill('SIGKILL'); reject(new Error('mint-timeout')); }, 120000);
    child.on('exit', code => {
      clearTimeout(timer);
      const m = out.match(/mint result: token len (\d+)/);
      if (code === 0 && m) resolve(parseInt(m[1], 10));
      else reject(new Error('mint failed (exit ' + code + '): ' + out.slice(-300)));
    });
    child.on('error', reject);
  });
}

const tokenLen = await mint();
const token = fs.readFileSync(tokenFile, 'utf8').trim();
if (token.length !== tokenLen) {
  console.error('token length mismatch: engine said ' + tokenLen + ', file has ' + token.length);
  process.exit(1);
}
console.log('minted token len', token.length, 'at', editorUrl);

const body = JSON.stringify({
  projectId, featureCategory: 'agents', featureName: 'agents',
  model: 'google/gemini-2.5-flash', step: 'message',
  sessionId: 'nb-' + Math.random().toString(16).slice(2, 8), stepIndex: 0,
  turnId: Array.from({ length: 32 }, () => Math.floor(Math.random() * 16).toString(16)).join(''),
  userPrompt: 'canary', castleToken: token,
  request: { model: 'google/gemini-2.5-flash', messages: [{ role: 'user', content: 'ok' }], max_tokens: 10, stream: false },
});

const spec = {
  addr: 'api.framer.com:443',
  host: 'api.framer.com',
  path: '/ai/v3/messages/',
  headers: {
    'accept': '*/*',
    'accept-encoding': 'gzip, deflate, br, zstd',
    'accept-language': 'en-US,en;q=0.9',
    'content-type': 'application/json; charset=utf-8',
    'origin': 'https://framer.com',
    'referer': editorUrl,
    'sec-ch-ua': '"Chromium";v="155", "Google Chrome";v="155", "Not_A Brand";v="99"',
    'sec-ch-ua-mobile': '?0',
    'sec-ch-ua-platform': '"Linux"',
    'sec-fetch-dest': 'empty',
    'sec-fetch-mode': 'cors',
    'sec-fetch-site': 'same-site',
    'user-agent': 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36',
    'x-requested-by': 'Framer',
    'authorization': 'Bearer ' + bearer,
    'cookie': cookie,
  },
  body,
};
const out = path.join(here, '..', 'tls_sender', 'live_request.json');
fs.writeFileSync(out, JSON.stringify(spec, null, 1), { mode: 0o600 });
console.log('wrote', out, 'bodyBytes', body.length);
console.log('handoff ready. The authorized live shot is:');
console.log('  cd scripts/tls_sender && LIVE_REQUEST_FILE=live_request.json ./tlssender');
