// Original pure Qon reducer and serializer, without a worker or SDK startup.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, literalBindings, sourceFunction, sourceBetween, decode} from './castle_v3_reference.mjs';

const names = `_2 Rn ct Nr It Ec wo Z E1 Nt h3 Ef sf x tn K3 f1 mo ur N3 In
gu sr Vn J3 C un l0 _r Sc ve Rt Ni er m3 q1 pt j0 qe kn Gt wt Xn Kr Zn _n Gr`
  .split(/\s+/);
const input = readFileSync(0, 'utf8');
if (sourceBetween('Dcn=', ',') !== 'Dcn=F') throw new Error('Unbound worker slot');
const program = `${preamble()}
const ${literalBindings(names)};
const Dcn=F;
const reduce=(${sourceFunction(506337)});
const serialize=(${sourceFunction(274985)});
const decode=(${decode.toString()});
const pairs=[
  [[_2,Rn,ct,t],[Nr,It,m,v]], [[Ec,wo,Z,t],[E1,Nt,M,v]],
  [[h3,Ef,b,o],[sf,x,tn,v]], [[K3,f1,rt,t],[mo,ur,nn,t]],
  [[N3,In,tn,u],[gu,sr,Vn,o]], [[J3,C,un,v],[l0,_r,b,u]],
  [[Sc,ve,Rt,t],[Ni,er,ct,t]], [[m3,q1,pt,t],[j0,qe,kn,t]]
].map(pair=>pair.map(args=>j(...args)));
({pairs, invalid:j(Gt,wt,Xn,t), tag:j(Kr,Zn,_n,u), empty:j(Gr,N,fn,o),
  bytes:JSON.parse(input).map(value=>Array.from(serialize([reduce(decode(value))])))})`;
const output = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify(output));
