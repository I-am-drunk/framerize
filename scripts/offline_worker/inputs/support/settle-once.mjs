import { performance } from 'node:perf_hooks';

const NAMES = new Set(['Error', 'TypeError', 'ReferenceError', 'RangeError',
  'SyntaxError', 'URIError', 'EvalError', 'DOMException', 'AbortError',
  'SecurityError', 'NotSupportedError']);
export function errorName(error) {
  try {
    const name = error?.name;
    return typeof name === 'string' && NAMES.has(name) ? name : 'unclassified';
  } catch { return 'unclassified'; }
}

// Invoke immediately and once. Never return the SDK's opaque value.
export function settleOnce(invoke, budgetMs) {
  if (typeof invoke !== 'function' || !Number.isFinite(budgetMs) || budgetMs <= 0)
    throw new TypeError('Invalid invocation or deadline');
  const started = performance.now();
  return new Promise(resolve => {
    const metadata = { calls: 0, late_settlements: 0,
      late_outcome: null, late_error_name: null };
    let finished = false, thenSettled = false, timer;
    const finish = (outcome, details = {}) => {
      if (finished) {
        metadata.late_settlements++;
        metadata.late_outcome = outcome;
        if (details.error_name) metadata.late_error_name = details.error_name;
        return;
      }
      finished = true;
      const elapsed_ms = performance.now() - started;
      Object.assign(metadata, details, { elapsed_ms,
        outcome: elapsed_ms > budgetMs && outcome !== 'timeout'
          ? 'deadline-exceeded' : outcome,
        observed_outcome: outcome === 'timeout' ? 'pending' : outcome });
      clearTimeout(timer);
      resolve(metadata);
    };
    timer = setTimeout(() => finish('timeout'), budgetMs);
    let returned, then;
    try { metadata.calls = 1; returned = invoke(); }
    catch (error) {
      finish('synchronous-throw', { error_name: errorName(error) });
      return;
    }
    try { then = returned?.then; }
    catch (error) {
      finish('then-access-threw', { error_name: errorName(error) });
      return;
    }
    if (typeof then !== 'function') {
      finish('unexpected-non-thenable', { return_type: typeof returned,
        nonempty_string: typeof returned === 'string' && returned.length > 0 });
      return;
    }
    const fulfill = value => {
      if (thenSettled) return;
      thenSettled = true;
      finish('fulfilled', { return_type: typeof value,
        nonempty_string: typeof value === 'string' && value.length > 0 });
    };
    const reject = error => {
      if (thenSettled) return;
      thenSettled = true;
      finish('rejected', { error_name: errorName(error) });
    };
    // Attach even after a synchronous overrun. Late rejections stay handled.
    try { Reflect.apply(then, returned, [fulfill, reject]); }
    catch (error) { reject(error); }
  });
}
