// Execute only the pinned SDK's self-contained compression fallback.
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import vm from 'node:vm';

const sdk = readFileSync(new URL('../reference/decompiled/castle/castleSDKV3-LNGA36I3.mjs', import.meta.url));
const sha256 = '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71';
if (createHash('sha256').update(sdk).digest('hex') !== sha256) {
  throw new Error('SDK differs from the reviewed source');
}
const input = JSON.parse(readFileSync(0, 'utf8'));
const compressor = sdk.subarray(575295, 581410).toString('utf8');
const program = `${compressor};
Promise.all(input.map(bytes => $2n(Uint8Array.from(bytes)).then(Array.from)));`;
// No CompressionStream/Blob/Response is supplied, selecting the real fallback.
const output = await vm.runInNewContext(program, {input}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify({sdk_sha256: sha256, backend: 'fallback', bytes: output}));
