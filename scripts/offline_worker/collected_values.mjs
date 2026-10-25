// Copy plain collector data without invoking property getters or conversion hooks.
import {types} from 'node:util';

function inheritedIndex(value, index) {
  let prototype = Object.getPrototypeOf(value), hops = 0;
  while (prototype !== null) {
    if (++hops > 64 || types.isProxy(prototype)) throw new Error('collector-prototype');
    if (Object.getOwnPropertyDescriptor(prototype, index)) return true;
    prototype = Object.getPrototypeOf(prototype);
  }
  return false;
}

export function capture(value, state = {seen:new Set(), count:0, units:0}, depth = 0) {
  if (++state.count > 4096 || depth > 64) throw new Error('collector-capture-limit');
  if (value === undefined) return {kind:'undefined'};
  if (value === null) return {kind:'null'};
  if (typeof value === 'boolean') return {kind:'boolean', value};
  if (typeof value === 'number')
    return {kind:'number', value:Object.is(value, -0) ? '-0' : String(value)};
  if (typeof value === 'string') {
    state.units += value.length;
    if (state.units > 65536) throw new Error('collector-string-limit');
    return {kind:'string', units:Array.from({length:value.length}, (_,i)=>value.charCodeAt(i))};
  }
  if (types.isProxy(value) || !Array.isArray(value) || state.seen.has(value))
    throw new Error('collector-data-shape');
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const length = descriptors.length.value;
  if (!Number.isSafeInteger(length) || length < 0 || length > 2048)
    throw new Error('collector-array-limit');
  state.seen.add(value);
  const values = Array.from({length}, (_,i)=>{
    const descriptor = descriptors[i];
    if (descriptor && !Object.hasOwn(descriptor,'value')) throw new Error('collector-accessor');
    if (!descriptor && inheritedIndex(value, i)) throw new Error('collector-inherited-index');
    return capture(descriptor?.value, state, depth+1);
  });
  state.seen.delete(value);
  return {kind:'array', values};
}
