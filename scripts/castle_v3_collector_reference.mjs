// Original collector-result assignments and their pure reducers only.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, literalBindings, sourceFunction, sourceBetween, decode} from './castle_v3_reference.mjs';

const names = `_2 Rn ct Nr It Ec wo Z E1 Nt h3 Ef sf x tn K3 f1 mo ur N3 In
gu sr Vn J3 C un l0 _r Sc ve Rt Ni er m3 q1 pt j0 qe kn Gt wt Xn Kr Zn _n Gr
F1 Wf g p T q _o _C Ie co Bn Z2`.split(/\s+/);
const unique = [...new Set(names)];
const worker = sourceBetween('Q[_o]=l(s,Qon(W))', ',Q[_T]');
const network = sourceBetween('Q[473]=a(X0,krn(W))', ',Q[Rp]');
const hash = sourceBetween('Q[_C]=l(k4,Brn(W))', ',Q[473]');
const input = readFileSync(0, 'utf8');
const program = `${preamble()}
const ${literalBindings(unique)};
const Dcn=F;
${sourceBetween('function Di(', 'function i4(')}
${sourceBetween('function p4(', 'function id(')}
${sourceBetween('function i1(', 'function ')}
${sourceBetween('function Cr(', 'function Nn(')}
const Qon=(${sourceFunction(506337)}),krn=(${sourceFunction(428839)});
const Brn=(${sourceFunction(429663)});
const serialize=(${sourceFunction(274985)});
const decode=(${decode.toString()});
JSON.parse(input).map(item=>{
  const Q=Array.from({length:765},(_,i)=>i+1000),W=decode(item.value);
  try {
    if(item.kind==='worker'){${worker};}
    else if(item.kind==='network'){${network};}else{${hash};}
    return {bytes:Array.from(serialize(item.compact?[Q[_C]]:Q))};
  } catch(error) {return {error:error.name,bytes:Array.from(serialize(Q))};}
})`;
const output = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify(output));
