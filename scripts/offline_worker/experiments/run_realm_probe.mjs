import * as pins from '../pins.mjs';
import {createRealmWindow} from '../realm_window.mjs';
import { PROBE } from './castle_probe.mjs';
import vm from 'node:vm';
const dom=createRealmWindow(pins.PUBLIC_KEY, undefined);
const ctx=dom.getInternalVMContext();
const out=await vm.runInContext(PROBE, ctx, {timeout: 15000});
console.log(JSON.stringify(out, null, 1));
await dom.close();
