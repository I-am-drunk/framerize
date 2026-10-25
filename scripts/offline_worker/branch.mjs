import { types } from 'node:util';

function own(object, key) {
  if (object === null || !['object', 'function'].includes(typeof object)) return;
  const field = Object.getOwnPropertyDescriptor(object, key);
  return field && Object.hasOwn(field, 'value') ? field.value : undefined;
}

export function observeBranch(entries) {
  const length = own(entries, 'length');
  if (!Array.isArray(entries) || !Number.isInteger(length) || length > 1000)
    throw new Error('logger-shape');
  const counts = { v3: 0, v2: 0, none: 0, errors: 0 };
  for (let index = 0; index < length; index++) {
    const entry = own(entries, String(index));
    if (own(own(entry, 'logger'), 'id') !== 'castle') continue;
    const level = own(entry, 'level');
    if (level === 4) counts.errors++;
    if (level !== 1) continue;
    const parts = own(entry, 'parts');
    if (!Array.isArray(parts) || own(parts, 'length') !== 1) continue;
    const message = own(parts, '0');
    if (message === 'Using Castle v3 token') counts.v3++;
    else if (message === 'Using Castle v2 token') counts.v2++;
    else if (message === 'No Castle token available') counts.none++;
  }
  return counts;
}

export function exactV3(counts) {
  return counts.v3 === 1 && counts.v2 === 0 && counts.none === 0 && counts.errors === 0;
}

const V3_FAILURES = [
  ['module load', 'error', 'v3-module-load-error'],
  ['module load', 'timeout', 'v3-module-load-timeout'],
  ['configuration', 'error', 'v3-configuration-error'],
  ['configuration', 'timeout', 'v3-configuration-timeout'],
  ['token generation', 'error', 'v3-token-generation-error'],
  ['token generation', 'timeout', 'v3-token-generation-timeout'],
  ['SDK', 'error', 'v3-sdk-error'],
];

function diagnosticOwn(object, key) {
  return types.isProxy(object) ? undefined : own(object, key);
}

function taggedFailure(tags) {
  if (tags === null || typeof tags !== 'object' || types.isProxy(tags)
      || Array.isArray(tags) || diagnosticOwn(tags, 'castleVersion') !== 'v3') return;
  const phase = diagnosticOwn(tags, 'castlePhase');
  const failure = diagnosticOwn(tags, 'castleFailure');
  return V3_FAILURES.find(([p, f]) => phase === p && failure === f)?.[2];
}

export function observeFailurePhase(entries) {
  try {
    const length = diagnosticOwn(entries, 'length');
    if (!Array.isArray(entries) || !Number.isInteger(length) || length > 1000) return;
    // Agreement describes emitted tags, not the first cause or proof of a timer firing.
    let selected;
    for (let index = 0; index < length; index++) {
      const entry = diagnosticOwn(entries, String(index));
      const id = diagnosticOwn(diagnosticOwn(entry, 'logger'), 'id');
      if (typeof id !== 'string') return;
      if (id !== 'castle') continue;
      const level = diagnosticOwn(entry, 'level');
      if (!Number.isInteger(level)) return;
      if (level !== 4) continue;
      const parts = diagnosticOwn(entry, 'parts');
      const size = diagnosticOwn(parts, 'length');
      if (!Array.isArray(parts) || !Number.isInteger(size) || size < 1 || size > 3) return;
      // reportError places its structured tags last. Never inspect earlier parts.
      const label = taggedFailure(diagnosticOwn(parts, String(size - 1)));
      if (!label || (selected !== undefined && selected !== label)) return;
      selected = label;
    }
    return selected;
  } catch { return undefined; }
}

export function refineFailurePhase(phase, label) {
  return (phase === 'mint' || phase === 'branch')
    && V3_FAILURES.some(([, , candidate]) => candidate === label) ? label : phase;
}
