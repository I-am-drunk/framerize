// Active offline diagnostic. Inspector may format Errors or invoke their hooks.
// Do not replace formatters. No Debugger, worker, listener, or raw-text export.
import inspector from 'node:inspector';
import { types } from 'node:util';
import { performance } from 'node:perf_hooks';

const HOLDER = '__framerizeV3ErrorLocation';
const GROUP = 'framerize-v3-error-location';
const { getPrototypeOf, getOwnPropertyDescriptor, defineProperty, hasOwn } = Object;
const { isNativeError, isProxy } = types;
const CLASSES = new Set(['error', 'type-error', 'range-error', 'reference-error',
  'syntax-error', 'uri-error', 'eval-error', 'aggregate-error']);
const ROLES = new Set(['fixture-host', 'fixture-vm', 'wrapper', 'timeout',
  'logger', 'services', 'bundler', 'v3']);

export class LocationFailure extends Error {
  constructor(code, stage) { super(code); this.code = code; this.stage = stage; }
}
function requireValue(ok, code, stage) {
  if (!ok) throw new LocationFailure(code, stage);
}

export function createErrorLocationReader() {
  const session = new inspector.Session();
  const cleanup = { group_released: false, console_discarded: false,
    runtime_disabled: false, session_disconnected: false, holder_removed: true };
  let connected = false, enabled = false, holder = null;
  let stage = 'holder-create';
  function post(method, params, at, deadline = Infinity) {
    stage = at;
    return new Promise((resolve, reject) => {
      const remaining = Math.min(500, deadline - performance.now());
      if (remaining <= 0 && at !== 'group-release') {
        reject(new LocationFailure('query-deadline', at));
        return;
      }
      let complete = false;
      const timer = setTimeout(() => {
        if (complete) return;
        complete = true;
        reject(new LocationFailure('query-deadline', at));
      }, Math.max(1, remaining));
      const done = (error, result) => {
        if (complete) return;
        complete = true;
        clearTimeout(timer);
        // Never retain or stringify a protocol error or its description.
        if (performance.now() >= deadline) reject(new LocationFailure('query-deadline', at));
        else if (error) reject(new LocationFailure(at, at)); else resolve(result);
      };
      try { session.post(method, params, done); } catch { done(true); }
    });
  }
  function closedFailure(error) {
    return error instanceof LocationFailure ? error
      : new LocationFailure('unexpected', stage);
  }
  async function enable() {
    try {
      requireValue(!connected && !holder && !hasOwn(globalThis, HOLDER),
        'holder-conflict', 'holder-create');
      holder = [null];
      defineProperty(globalThis, HOLDER, { value: holder, configurable: true });
      cleanup.holder_removed = false;
      stage = 'connect';
      session.connect();
      connected = true;
      await post('Runtime.enable', {}, 'runtime-enable');
      enabled = true;
    } catch (error) { throw closedFailure(error); }
  }
  function classOf(value, prototypes) {
    if (isProxy(value) || !isNativeError(value)) return null;
    const prototype = getPrototypeOf(value);
    const known = prototypes.find(entry => entry.prototype === prototype);
    return known && CLASSES.has(known.class_tag) ? known.class_tag : null;
  }
  function locationOf(details, sources) {
    // First allowlisted URL-matched frame, possibly a caller. Not script identity
    // or proof of the throwing instruction; null means no bounded match.
    const frames = details?.exceptionDetails?.stackTrace?.callFrames;
    if (!Array.isArray(frames)) return null;
    for (let index = 0; index < Math.min(frames.length, 16); index++) {
      const frame = frames[index];
      const source = sources.find(entry => entry.url === frame?.url);
      if (!source || !ROLES.has(source.source_role)) continue;
      const line = frame.lineNumber, column = frame.columnNumber;
      if (!Number.isInteger(line) || !Number.isInteger(column)
          || line < 0 || line >= source.lines.length || column < 0
          || column > source.lines[line].length) continue;
      return { source_role: source.source_role, line_number: line, column_number: column };
    }
    return null;
  }
  async function read(error, { prototypes, sources }) {
    // Shared deadline across the chain, including release. Same-thread work can
    // delay timers, so the outer process limit remains the hard runtime bound.
    const deadline = performance.now() + 1000;
    const result = { errors: [], cause_stop: 'absent' };
    let failure = null, value = error;
    const seen = new Set();
    try {
      stage = 'select-error';
      requireValue(enabled && Array.isArray(prototypes) && prototypes.length <= 32
        && Array.isArray(sources) && sources.length <= 8, 'select-error', stage);
      for (let depth = 0; depth < 4; depth++) {
        const classTag = classOf(value, prototypes);
        requireValue(classTag !== null, 'select-error', stage);
        seen.add(value);
        holder[0] = value;
        let remote = await post('Runtime.evaluate', {
          expression: 'globalThis.__framerizeV3ErrorLocation[0]', objectGroup: GROUP,
          includeCommandLineAPI: false, silent: true, returnByValue: false,
          generatePreview: false,
        }, 'remote-evaluate', deadline);
        stage = 'remote-type';
        requireValue(!remote.exceptionDetails && remote.result?.type === 'object'
          && remote.result?.subtype === 'error'
          && typeof remote.result?.objectId === 'string', 'remote-type', stage);
        const objectId = remote.result.objectId;
        remote = null;
        let details = await post('Runtime.getExceptionDetails',
          { errorObjectId: objectId }, 'exception-details', deadline);
        stage = 'location';
        result.errors.push({ cause_depth: depth, class_tag: classTag,
          location: locationOf(details, sources) });
        details = null;
        holder[0] = null;
        stage = 'select-error';
        const cause = getOwnPropertyDescriptor(value, 'cause');
        if (!cause) break;
        if (!hasOwn(cause, 'value')) { result.cause_stop = 'accessor'; break; }
        const next = cause.value;
        if (isProxy(next) || !isNativeError(next)) {
          result.cause_stop = 'non-native'; break;
        }
        if (!classOf(next, prototypes)) { result.cause_stop = 'unknown-prototype'; break; }
        if (seen.has(next)) { result.cause_stop = 'cycle'; break; }
        if (depth === 3) { result.cause_stop = 'depth-limit'; break; }
        value = next;
      }
    } catch (error) { failure = closedFailure(error); }
    finally {
      if (holder) holder[0] = null;
      value = null;
      seen.clear();
      try { await post('Runtime.releaseObjectGroup', { objectGroup: GROUP }, 'group-release', deadline); }
      catch (error) { failure ??= closedFailure(error); }
    }
    if (failure) throw failure;
    return result;
  }
  async function close() {
    if (holder) {
      holder[0] = null;
      try {
        cleanup.holder_removed = Reflect.deleteProperty(globalThis, HOLDER)
          && !hasOwn(globalThis, HOLDER);
      } catch { cleanup.holder_removed = false; }
      holder = null;
    }
    if (connected) {
      for (const [method, params, key] of [
        ['Runtime.releaseObjectGroup', { objectGroup: GROUP }, 'group_released'],
        ['Runtime.discardConsoleEntries', {}, 'console_discarded'],
        ['Runtime.disable', {}, 'runtime_disabled'],
      ]) {
        try { await post(method, params, 'cleanup'); cleanup[key] = true; }
        catch { cleanup[key] = false; }
      }
      try { session.disconnect(); cleanup.session_disconnected = true; }
      catch { cleanup.session_disconnected = false; }
    } else { cleanup.session_disconnected = true; }
    connected = false;
    enabled = false;
    return cleanup;
  }
  return { enable, read, close, cleanup };
}
