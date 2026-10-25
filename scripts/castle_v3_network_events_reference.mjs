// Source-exact pure RTC normalization and reduction, with supplied events only.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, literalBindings, sourceBetween, sourceFunction} from './castle_v3_reference.mjs';

const input = readFileSync(0, 'utf8');
const names = ('$ Tn Lp i5 gr j2 q3 mr y1 j8 dh C8 Tr pt Wf ht In p Lf pr qt Bn ' +
  'B2 T2 Zo lh Ae kn F4 ok C0 Z zf ur C Kt Ft Sn Gv un x Dn Rn ' +
  '$n rn Ry Jn En sh Hy By wi Fv XT eu Z1 hr Oh xT ot dj Zj').split(' ');
const helpers = [
  sourceBetween('function cn(', 'function zc('),
  sourceBetween('function Mr(', 'function r3('),
  sourceBetween('function $i(', 'function C2('),
  sourceBetween('function so(', 'function H5('),
  sourceBetween('function od(', 'function p4('),
].join('\n');
const normalizer = sourceBetween('function(Qn){var _=[];', '})(G),Cn=a(zt,S)') + '}';
const deduplicate = sourceBetween('Cn=a(zt,S);!R[Cn]', ';v');
const sdp = sourceBetween('if(sn&&c(s,typeof sn[Oh],Et,Xt,fn,v)){var et=', 'return e[l(wr,"cwB');
const reduceSource = `function reduce(events) {
  const Y=[],R={},histories=[[],[],[]];
  for (const event of events) {
    if (event.kind==='candidate') {
      const value=event.value;
      if ('port' in value) value.port=Number(value.port);
      var S=normalize(value),Cn;
      ${deduplicate};
    } else if (event.kind==='state') {
      c(S0,histories[event.group],{value:event.value},'value');
    } else if (event.kind==='sdp') {
      var G,J,Qn,_,W,on,Hn,Wn,tt=j(eu,Z1,hr,o),sn={sdp:event.value};
      ${sdp}
    } else throw new Error('Unknown fixture event');
  }
  const result=finish('192.0.2.1','',Y,...histories);
  return {bytes:Array.from(serialize([result])),order:result[0][p].map(row=>{
    const value=a(zt,row);
    return Array.from({length:value.length},(_,i)=>value.charCodeAt(i));
  })};
}`;
const program = `${preamble()}
const ${literalBindings(names)};
const ${sourceBetween('PW=/', ',T3n=')};
const ${sourceBetween('B3n=/', ',La=')};
const ${sourceBetween('LT=l(81,', ',dh=')};
${helpers}
const normalize=(${normalizer});
const finish=(${sourceBetween('function(Wn,tt,et,yt,Ot,Bt)', ')(...(Cn=w[t]')});
const serialize=(${sourceFunction(274985)});
${reduceSource}
const data=JSON.parse(input);
({classes:data.addresses.map(od), ports:data.ports.map(value=>IP(Number(value))),
  observations:data.cases.map(events=>reduce(events))});
`;
const result = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 5000, contextCodeGeneration: {strings:false, wasm:false},
});
process.stdout.write(JSON.stringify({...result, unicode:process.versions.unicode}));
