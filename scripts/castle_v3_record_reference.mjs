// Original N_ finalizer, supplied with synthetic scalar candidate records.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, literalBindings, sourceBetween, sourceFunction, decode} from './castle_v3_reference.mjs';

const input = readFileSync(0, 'utf8');
const names = '$ Tn Lp i5 gr j2 q3 mr y1 j8 dh C8 Tr pt Wf ht In p Lf pr qt Bn ST Sn'.split(' ');
function candidate(fields) {
  const paths = [[0,5,6],[1,7],[2,5,3,3],[1,4,2,4],[2,2,1,6],[0,10],[0,8,8,5]];
  const record = [];
  paths.forEach((path, index) => {
    let owner = record;
    for (const key of path.slice(0,-1)) owner = owner[key] ??= [];
    owner[path.at(-1)] = decode(fields[index]);
  });
  return record;
}
const program = `${preamble()}
const ${literalBindings(names)};
${sourceBetween('function UP(', 'function hZ(')}
${sourceBetween('function hZ(', 'function p4(')}
${sourceBetween('function xP(', 'function od(')}
const finish=(${sourceBetween('function(Wn,tt,et,yt,Ot,Bt)', ')(...(Cn=w[t]')});
const serialize=(${sourceFunction(274985)});
const decode=(${decode.toString()});
const candidate=(${candidate.toString()});
({fields:[Wf,ht,In,p,Pn,Lf,pr,qt,m],
unavailable:Array.from(serialize([xP()])), rows:JSON.parse(input).map(item=>{
  const result=finish(item.address,item.related,item.candidates.map(candidate),...item.histories);
  const order=result[0][p].map(row=>{
    const text=a(zt,row);
    return Array.from({length:text.length},(_,index)=>text.charCodeAt(index));
  });
  return {order,bytes:Array.from(serialize([result]))};
})})`;
const output = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify(output));
