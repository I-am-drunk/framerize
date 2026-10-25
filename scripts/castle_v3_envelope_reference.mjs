// Offline envelope reference with an explicit synthetic clock.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, sourceBetween, sourceFunction, literalBindings} from './castle_v3_reference.mjs';

const names = `
$ $e $n Ai B1 Bn Bu C3 D1 Dn EO Ef En Eo Ie If In J1 JH Je Jn Jt Le Lf Me
Mf Mi Ne Nf Oo Qe Qu Re Rn S1 Si Tn Ue V1 Ve Xf Xu Y1 Yf Yu Z Z2 Zn _e
a1 ae b0 c0 ce cf ci co de dh dt gi hf hn hr ht kn l1 mf mi of ot p pf pr
pu q1 qr qt rn ru t1 tK tn un we x xe xn xu yf yi`.trim().split(/\s+/);
const declarations = literalBindings(names);
const calculated = sourceBetween('jE=l(81,128,8,91,16,93,8,172,0)', ',TE=');
const encodeTime = sourceBetween('function Cr(n)', 'function Nn(');
const helpers = sourceBetween('function cn(n,r,e,w,i,D,A)', 'function af(')
  + sourceBetween('function At(n,r,e,w,i)', 'function uc(');
const envelope = sourceFunction(276403);
const input = readFileSync(0, 'utf8');
const program = `
let clock=0;
const Date={now:()=>clock};
${preamble()}
const ${declarations};
const ${calculated};
${encodeTime}
${helpers}
const envelope=(${envelope});
const request=JSON.parse(input);
request.cases.map(item=>{
  clock=item.timestamp;
  const bytes=Uint8Array.from(item.data);
  return {value:envelope(bytes)};
});`;
const output = vm.runInNewContext(program, {input, btoa, atob, TextEncoder}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify(output));
