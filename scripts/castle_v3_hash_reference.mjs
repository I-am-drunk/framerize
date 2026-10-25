// Original pure UP hash, without the SDK module or network collection.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {preamble, literalBindings, sourceBetween, decode} from './castle_v3_reference.mjs';

const input = readFileSync(0, 'utf8');
const program = `${preamble()}
const ${literalBindings(['$', 'Tn', 'Lp', 'i5'])};
${sourceBetween('function UP(', 'function hZ(')}
const decode=(${decode.toString()});
({aliases:[$,Tn,Lp,i5], rows:JSON.parse(input).map(value=>{
  try { return {hash:UP(decode(value))}; }
  catch (error) { return {error:error.name}; }
})})`;
const output = vm.runInNewContext(program, {input, atob, btoa}, {
  timeout: 5000, contextCodeGeneration: {strings: false, wasm: false},
});
process.stdout.write(JSON.stringify(output));
