import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { NativeContext } from './egl.mjs';

const cycles = [];
for (let i = 0; i < 2; ++i) {
  const context = new NativeContext();
  try {
    assert.throws(() => new NativeContext(), /already owned/);
    const result = context.query();
    assert.match(result.renderer, /llvmpipe/);
    assert.match(result.version, /OpenGL ES/);
    assert.ok(result.vendor && result.extensions);
    cycles.push({ egl_version: context.version, software_renderer: true,
      fields_sha256: Object.fromEntries(Object.entries(result).map(([k, v]) =>
        [k, createHash('sha256').update(v).digest('hex')])),
      extension_count: result.extensions.split(' ').filter(Boolean).length });
  } finally { context.close(); }
  assert.throws(() => context.query(), /context closed/);
  context.close();
  cycles.at(-1).context_closed = true;
}
fs.writeFileSync('/out/probe.json', JSON.stringify({ passed: true, cycles,
  sdk_executed: false, webgl_implemented: false, shader_executed: false,
  exclusive_owner_refusal: true, closed_query_refusal: true,
  idempotent_close: true }, null, 2) + '\n', { flag: 'wx' });
