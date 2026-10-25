// Offline reference: selected pure helpers, never the SDK module or its startup.
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import vm from 'node:vm';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const sdk = readFileSync(new URL('../reference/decompiled/castle/castleSDKV3-LNGA36I3.mjs', import.meta.url));
const sha256 = '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71';
if (createHash('sha256').update(sdk).digest('hex') !== sha256) {
  throw new Error('SDK differs from the reviewed source');
}
const source = sdk.toString('utf8');
const slice = (start, end) => sdk.subarray(start, end).toString('utf8');
const between = (start, end) => {
  const at = source.indexOf(start);
  const stop = source.indexOf(end, at + start.length);
  if (at < 0 || stop < 0) throw new Error('Missing pinned source boundary');
  return source.slice(at, stop);
};
const enums = between('var fA={__proto__:null},', ';let z=Math') + ';';
const builtins = between('let z=Math,', ',Rc=') + ';';
const decoder = between('function E(n)', 'function cn(');
const textDecoder = between('function j(n,r,e,w)', 'function FL(');
const table = between('WZ=[', ',Ux=[]');
const dispatch = slice(2702, 16321);
const nullChecks = between('function he(n)', 'function Xw(');
const serializer = slice(274985, 276393);
const constants = {
  Un: '', Be: 42, h7: 1023, xf: 96, no: 'join', I: 'length', Qi: 'charCodeAt',
  t: 0, o: 1, u: 2, v: 3, h: 4, b: 5, B: 6, M: 7, f: 8, U: 9, F: 10,
  ee: 128, N: 127, at: 255, nn: 31, y: 16, Pn: 24,
  Et: 90, Xt: 236, fn: 19, fu: 439, m: 47, rt: 39, ly: 942, me: 149,
  Jo: 'test', ye: 'isFinite', Pi: 'floor', U3: 'isArray', z2: 'stringify',
  yC: 'buffer', MM: 'setFloat64',
};
const literalBlock = slice(314400, 316640);
for (const [name, value] of Object.entries(constants)) {
  const literal = `${name}=${JSON.stringify(value)}`;
  if (!literalBlock.includes(`,${literal},`) && !literalBlock.includes(`;${literal},`)) {
    throw new Error(`Unbound SDK constant: ${name}`);
  }
}
if (!literalBlock.includes('gM=-1>>>0,')) throw new Error('Unbound u32 limit');
const declarations = Object.entries(constants)
  .map(([name, value]) => `${name}=${JSON.stringify(value)}`).join(',');

export function preamble() {
  return `${enums}\n${builtins}\n${decoder}\n${dispatch}\n${nullChecks}
const ${declarations}; const gM=-1>>>0;
let ${table},Ux=[]; const uK=String.fromCharCode; ${textDecoder}`;
}

export function sourceBetween(start, end) { return between(start, end); }

export function sourceFunction(offset) {
  const text = sdk.subarray(offset).toString('utf8');
  if (!text.startsWith('function(')) throw new Error('Not a function boundary');
  let depth = 0;
  for (let index = text.indexOf('{'); index < text.length; index++) {
    if ('\'"`'.includes(text[index])) throw new Error('Quoted function needs a parser');
    if (text[index] === '{') depth++;
    if (text[index] === '}' && --depth === 0) return text.slice(0, index + 1);
  }
  throw new Error('Unclosed function');
}

export function literalBindings(names) {
  return names.map(name => {
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const pattern = new RegExp(`(?<![\\w$])${escaped}=(-?\\d+(?:e\\d+)?|"[^"\\n]*")(?=[,;])`, 'g');
    const values = [...new Set([...source.matchAll(pattern)].map(match => match[1]))];
    if (values.length !== 1) throw new Error(`Ambiguous literal: ${name}`);
    return `${name}=${values[0]}`;
  }).join(',');
}

function decode(value) {
  switch (value.kind) {
    case 'null': return null;
    case 'undefined': return undefined;
    case 'boolean': return value.value;
    case 'number': return Number(value.value);
    case 'string': return String.fromCharCode(...value.units);
    case 'array': return value.values.map(decode);
    case 'object': return Object.fromEntries(value.entries.map(([key, val]) => [key, decode(val)]));
    case 'function': return () => 0;
    case 'symbol': return Symbol('synthetic');
    case 'throwing': return {toJSON() { throw new Error('synthetic'); }};
    default: throw new Error('Unknown synthetic input');
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
const input = readFileSync(0, 'utf8');
const program = `
${preamble()}
const serialize=(${serializer});
const decode=(${decode.toString()});
const cases=JSON.parse(input);
({types:[j(Et,Xt,fn,v),j(fu,m,rt,o),j(ly,me,m,o)],
  bytes:cases.map(values=>Array.from(serialize(values.map(decode))))});`;
const result = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 3000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify({sdk_sha256: sha256, ...result}));
}
