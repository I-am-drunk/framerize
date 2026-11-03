import { randomInt } from 'node:crypto';
import { readCredential } from './config.js';
import { mintCredential } from './credentials.js';

const minted = new Map();
const ID_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';

function mintId() {
  return Array.from({ length: 9 }, () => ID_ALPHABET[randomInt(62)]).join('');
}

async function credentials(account, env) {
  const session = account.sessionEnv ? env[account.sessionEnv] : await readCredential(account.sessionFile);
  if (!session) {
    const error = new Error(`account ${account.id} has no session credential`);
    error.status = 503;
    throw error;
  }
  return { session };
}

async function bearer(account, fetchImpl, signal, env) {
  if (account.authorizationEnv) {
    const token = env[account.authorizationEnv];
    if (token) return token;
  }
  const cached = minted.get(account.id);
  if (cached?.expiresAt > Date.now() + 30_000) return cached.token;
  if (cached?.pending) return cached.pending;
  const pending = mintBearer(account, fetchImpl, signal, env);
  minted.set(account.id, { pending });
  try {
    const result = await pending;
    minted.set(account.id, result);
    return result.token;
  } catch (error) {
    if (minted.get(account.id)?.pending === pending) minted.delete(account.id);
    throw error;
  }
}

async function mintBearer(account, fetchImpl, signal, env) {
  const { session } = await credentials(account, env);
  const base = (account.upstream || 'https://api.framer.com').replace(/\/+$/, '');
  const response = await fetchImpl(`${base}/auth/ai/access-token`, {
    headers: { accept: 'application/json', cookie: session }, signal, redirect: 'error',
  });
  if (!response.ok) {
    const error = new Error(`Framer authentication returned HTTP ${response.status}`);
    error.status = response.status === 401 ? 401 : 502;
    throw error;
  }
  const data = await response.json();
  const expiresAt = Date.parse(data.expiresAt);
  if (!data.accessToken || !Number.isFinite(expiresAt)) throw new Error('invalid Framer token response');
  return { token: data.accessToken, expiresAt };
}

function userPrompt(payload) {
  const input = payload.input ?? payload.messages;
  const latest = Array.isArray(input) ? [...input].reverse().find((item) => item.role === 'user') : null;
  const content = latest?.content ?? input;
  const text = typeof content === 'string' ? content
    : Array.isArray(content) ? content.map((part) => part.text || '').join(' ') : '';
  return [...text].slice(0, 300).join('');
}

export async function prepareFramer({ account, path, body, signal, fetchImpl, env = process.env }) {
  if (!account || account.provider !== 'framer') return null;
  const castle = account.castleSocket ? await mintCredential(account.castleSocket, account.id)
    : account.castleEnv ? env[account.castleEnv] : await readCredential(account.castleFile);
  if (!castle) {
    const error = new Error(`account ${account.id} has no Castle credential`);
    error.status = 503;
    throw error;
  }
  const payload = JSON.parse(body.toString('utf8'));
  const token = await bearer(account, fetchImpl, signal, env);
  const destination = path === '/v1/responses' ? '/ai/v3/openai/responses/' : '/ai/v3/chat/';
  const base = (account.upstream || 'https://api.framer.com').replace(/\/+$/, '');
  return { url: `${base}${destination}`, authorization: `Bearer ${token}`, body: JSON.stringify({
    projectId: account.projectId, featureCategory: 'agents', featureName: 'agents',
    model: payload.model, step: 'message', sessionId: mintId(),
    stepIndex: 0, turnId: mintId(), userPrompt: userPrompt(payload),
    castleToken: castle, request: payload,
  }) };
}

