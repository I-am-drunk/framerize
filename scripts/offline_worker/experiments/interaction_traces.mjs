// Library of real captured interaction traces. Every trace was recorded live
// in the user's tab through the devtools Input domain (genuine trusted events,
// real timing); nothing is synthesized. pickTrace() chooses one at random per
// mint so the replayed behavior is not the identical path every time.
import fs from 'node:fs';

const TRACE_FILES = [
  './interaction-trace.json',
  './interaction-trace-long.json',
];

let TRACES = null;
function loadTraces() {
  if (TRACES) return TRACES;
  TRACES = [];
  for (const f of TRACE_FILES) {
    try {
      const t = JSON.parse(fs.readFileSync(new URL(f, import.meta.url), 'utf8'));
      if (Array.isArray(t.events) && t.events.length > 0) TRACES.push(t.events);
    } catch {}
  }
  return TRACES;
}

// One random real trace per mint. Falls back to [] when no capture exists.
export function pickTrace() {
  const traces = loadTraces();
  if (traces.length === 0) return [];
  return traces[Math.floor(Math.random() * traces.length)];
}

export function traceCount() {
  return loadTraces().length;
}
