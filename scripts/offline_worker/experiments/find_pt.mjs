import fs from 'node:fs';
import * as pins from '../pins.mjs';
import vm from 'node:vm';
// Load the v3 module's string table decoder to resolve what Pt maps to.
// Simplest: run the module far enough to read its globals is hard. Instead,
// decode statically: find "Pt" in the source and the string table.
const src = fs.readFileSync(pins.ASSETS + '/castleSDKV3-LNGA36I3.mjs', 'utf8');
// The string table is the giant array near the top. Find where Pt is defined.
// Look for the pattern: Pt followed by its value or the decoder usage.
// Print the region right before the failing code that defines n[6].
const line7 = src.split('\n')[6];
const i = line7.indexOf('n[7]=ef(n,Pt)');
console.log('context before n[7]=ef(n,Pt):');
console.log(line7.slice(Math.max(0, i - 300), i + 50));
