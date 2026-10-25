// Original pure network-record reducer. No RTC, worker or SDK startup.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, literalBindings, sourceFunction, sourceBetween, decode} from './castle_v3_reference.mjs';

const names = 'F1 Z Wf g p C Gr Nr It T q E1 Nt Kr Zn _n Gt wt Xn'.split(' ');
const input = readFileSync(0, 'utf8');
const program = `${preamble()}
const ${literalBindings(names)};
${sourceBetween('function Di(', 'function i4(')}
${sourceBetween('function p4(', 'function id(')}
const reduce=(${sourceFunction(428839)});
const serialize=(${sourceFunction(274985)});
const decode=(${decode.toString()});
({kinds:[j(u,F1,Z,v),j(g,p,C,u),j(T,q,m,t)], getter:typeof ud,
  rows:JSON.parse(input).map(value=>{
    try { return {bytes:Array.from(serialize([reduce(decode(value))]))}; }
    catch (error) { return {error:error.name}; }
  })})`;
const output = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify(output));
