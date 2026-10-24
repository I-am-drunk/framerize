import { createHash } from 'node:crypto';
import { SourceTextModule } from 'node:vm';

// Evaluate only the caller's pinned factory in this operation's own window.
// The module and SDK object stay local to the operation, never Node globals.
export async function loadV2RealmFactory(dom, bytes, digest) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > 1024 * 1024
      || typeof digest !== 'string' || !/^[a-f0-9]{64}$/.test(digest)
      || createHash('sha256').update(bytes).digest('hex') !== digest)
    throw new Error('factory-identity');
  const source = bytes.toString('utf8');
  if (!Buffer.from(source, 'utf8').equals(bytes)) throw new Error('factory-encoding');
  const refuseImport = () => { throw new Error('factory-import'); };
  const module = new SourceTextModule(source, {
    context: dom.getInternalVMContext(), identifier: 'framer-v2-factory.mjs',
    importModuleDynamically: refuseImport,
  });
  await module.link(refuseImport);
  await module.evaluate({ timeout: 1000 });
  if (typeof module.namespace.default !== 'function') throw new Error('factory-export');
  const sdk = new dom.window.Object();
  module.namespace.default(sdk);
  if (typeof sdk.configure !== 'function') throw new Error('factory-configure');
  return sdk;
}
