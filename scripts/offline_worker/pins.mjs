import { fileURLToPath } from 'node:url';
const ROOT = fileURLToPath(new URL('./inputs', import.meta.url));
export const GRAPHICS = null;
export const ASSETS = ROOT + '/assets';
export const DOM = ROOT + '/dom';
export const HISTORY = ROOT + '/history';
export const CDN = 'https://app.framerstatic.com/';
export const WRAPPER_MINT_EXPORT = 'a';
export const PUBLIC_KEY = 'pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB';
export const MODULES = Object.freeze({
  wrapper: ['chunk-5LKSDTQ5.mjs', ['timeout', 'logger', 'bundler'],
    'cf52d0c8822a441b149fbee55f6832eec006ee1c63cf5f8538159437fb072ef0'],
  timeout: ['chunk-DWEQS5LX.mjs', [],
    'c63499801763884751511fe681d405bdec72ff7d64fdfe004a5494ed4e852912'],
  logger: ['chunk-VJ7UYMJI.mjs', ['services'],
    '8ce67885ebb85b04f2ecebbfa1b3158e811f00615240f0706c29ecbb334a8eb9'],
  services: ['chunk-YRQ7G4QH.mjs', [],
    '1820d35d5bbf939ad80218c432221b27dd525cf657d244c098b026c94c585fa1'],
  bundler: ['chunk-WLHSDIGQ.mjs', [],
    '1e48053e96ee0c95360c3af0b4e6b8f3e824036d02df6776e641b80e98b57235'],
  v3: ['castleSDKV3-LNGA36I3.mjs', ['bundler'],
    '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71'],
});
export const SUPPORT = Object.freeze({
  settle: [ROOT + '/support/settle-once.mjs',
    '916164efde2363c572bef4f3c6e95a90d2ad4599b1da659685b556c6523fa73c'],
  location: [ROOT + '/support/error-location.mjs',
    'b40c831d1b65df86bd89e5604589e531f4f9c631f7dce487c02288dde8180238'],
  writer: [ROOT + '/support/write-token-once.mjs',
    '6da5ca013764a32c516afe8f01b5b6e5bac63b7b3c40397ef428bfa02275bddb'],
});
export const DOM_PINS = Object.freeze({
  'package.json': '7fb9926bbecb12bac874dcdeb3ad32af640bbbc66b7fa89c4b1788edfce64b95',
  'package-lock.json': '51b8973cdd703e23e9f632a23e9d80aaa1b52e515aa6f7f5fcfaab4acbf589ff',
  'node_modules/jsdom/package.json': '9670cfae0de2c46a3276fa20815328e3903275b04cc8969a6f6be01f2c580043',
  'node_modules/jsdom/lib/api.js': '0653e655065e93d44d885b04a975c0bac3221513bfc7be0be0afd2a4cc99ff2e',
  'node_modules/jsdom/lib/jsdom/browser/Window.js': 'cfd16270beb33c10564b3474fd2e184b902f66f35eee2cab9191c699e36b4287',
  'node_modules/jsdom/lib/jsdom/living/nodes/Document-impl.js': '28a1ac5b19463cc2db29b59d5721ce20299b998f00a222ac2ab74117dc66ba0d',
});
for (const values of Object.values(MODULES)) {
  Object.freeze(values[1]);
  Object.freeze(values);
}
for (const values of Object.values(SUPPORT)) Object.freeze(values);
