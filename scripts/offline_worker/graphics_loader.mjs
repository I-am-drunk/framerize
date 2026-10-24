import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';

function sha(file) {
  const hash = createHash('sha256'), buffer = Buffer.allocUnsafe(65536);
  const fd = fs.openSync(file, 'r');
  try {
    for (;;) {
      const count = fs.readSync(fd, buffer, 0, buffer.length, null);
      if (!count) break;
      hash.update(buffer.subarray(0, count));
    }
  } finally { fs.closeSync(fd); }
  return hash.digest('hex');
}

export function loadGraphics(spec) {
  if (!path.isAbsolute(spec.root) || fs.realpathSync(spec.root) !== spec.root)
    throw new Error('graphics root');
  const manifest = path.join(spec.root, 'files.json');
  if (fs.statSync(manifest).size > 1048576 || sha(manifest) !== spec.manifestSha256)
    throw new Error('graphics manifest');
  const entries = Object.entries(JSON.parse(fs.readFileSync(manifest, 'utf8')));
  if (!entries.length || entries.length > 4096) throw new Error('graphics files');
  const verify = () => {
    if (sha(manifest) !== spec.manifestSha256) throw new Error('graphics manifest');
    for (const [file, expected] of entries) {
      if (!path.isAbsolute(file) || !/^[a-f0-9]{64}$/.test(expected) || sha(file) !== expected)
        throw new Error('graphics input');
    }
  };
  verify();
  Object.assign(process.env, {
    EGL_PLATFORM: 'surfaceless', LIBGL_ALWAYS_SOFTWARE: '1',
    GALLIUM_DRIVER: 'llvmpipe', LP_NUM_THREADS: '0',
    LP_NATIVE_VECTOR_WIDTH: '128', MESA_SHADER_CACHE_DISABLE: 'true',
  });
  const require = createRequire(import.meta.url);
  const create = require(spec.root);
  const { NativeWebGL } = require(path.join(spec.root, 'src/javascript/native-gl.js'));
  return { create, verify, cleanup: () => NativeWebGL.cleanup(),
    activeContexts: () => NativeWebGL.activeContexts() };
}
