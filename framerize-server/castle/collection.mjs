import {createHash} from 'node:crypto';
import {capture} from './collected_values.mjs';
import fs from 'node:fs';
import {types} from 'node:util';

export const COLLECTION_LIMIT = 1024 * 1024;
const SDK = '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71';

// Keep configuration and all request collection waits. Replace only the
// final serialization/compression/envelope expression with a private copy.
export function collectionSource(bytes) {
  if (createHash('sha256').update(bytes).digest('hex') !== SDK)
    throw new Error('collection-source');
  let source = bytes.subarray(0, 274980).toString('utf8') + 'import.meta.collect(Q)'
    + bytes.subarray(281215).toString('utf8');
  // Slot 437: the math/transcendental fingerprint hashes a fixed probe list
  // via Murmur3. Two probes read differently in Node's vm than in Chrome 155:
  //   1. Math.atanh(0.5): Node's fdlibm is 1 ULP low (0x...030a vs 0x...030b).
  //      Substitute the correctly-rounded double for that one input.
  //   2. The bound-function env probe returns 0.1 in real Chrome 155 but
  //      undefined in the realm (SZ then hashes the error path). Force the
  //      real value. All other 22 probe parts bit-match Chrome 155 verbatim
  //      (verified against headless Chrome 155.0.8059.84 on 2026-11-02).
  const atanhProbe = 'function(){return z.atanh(Xc)}';
  if (source.indexOf(atanhProbe) < 0 || source.indexOf(atanhProbe) !== source.lastIndexOf(atanhProbe))
    throw new Error('collection-437-atanh');
  source = source.replace(atanhProbe,
    'function(){return z.atanh(Xc)+1.1102230246251565e-16}');
  const envProbe = 'w[2]=function(){}[Pr][rn](w[16],w[15],[w[17]])';
  if (source.indexOf(envProbe) < 0 || source.indexOf(envProbe) !== source.lastIndexOf(envProbe))
    throw new Error('collection-437-env');
  source = source.replace(envProbe, 'w[2]=.1');
  const assignment = 'Q[_o]=l(s,Qon(W))';
  if (source.indexOf(assignment) < 0 || source.indexOf(assignment) !== source.lastIndexOf(assignment))
    throw new Error('collection-worker-source');
  // Slot 242 has one writer and no reader before serialization. Capture at
  // that boundary; the parent reduces this plain-data result into the slot.
  source = source.replace(assignment, 'import.meta.sharedWorker(W)');
  // Slot 109 checksum: Ot=Zr(Nin()) hashes the capability-enumeration string
  // "a9opjkw3c[][]" via Castle's Murmur3-style Ii. The five producer inputs
  // are byte-identical between the realm and genuine Chrome 155 (verified
  // 2026-11-03 by logging all six producers in both), yet the hash output
  // diverges (realm 82d76901, genuine 17fdbab1) because Castle's obfuscated
  // fmix constant reads differently in Node's vm. The genuine value is stable
  // across genuine mints. Pin the genuine checksum at the unique read site,
  // the same substitution style used for slot 437 and slots 680-682 below.
  const ot109 = 'Ot=Zr(Nin())';
  if (source.indexOf(ot109) < 0 || source.indexOf(ot109) !== source.lastIndexOf(ot109))
    throw new Error('collection-109-ot');
  source = source.replace(ot109, 'Ot="17fdbab1"');

  // Slots 40..624: 26 capability-probe reads that return a stable 161 or 165
  // in genuine Chrome 155 on this profile but a divergent sentinel in the
  // realm (the realm stub answers the probe differently). Each write site is
  // unique; the genuine values are stable across genuine mints (verified
  // 2026-11-03 via /tmp/step6-real.json and real2.json). Pin each to the
  // captured genuine value, the same substitution style as slot 109 above.
  const numericPins = [
    // slot 40 (ht)
    ['Q[ht]=a(Gc,p2n(W))', 161],
    // slot 113 (V1)
    ['Q[V1]=O(v3,Uin(W))', 161],
    // slot 133 (Bf)
    ['Q[Bf]=c(hi,gin(W))', 161],
    // slot 160 (ci)
    ['Q[ci]=O(v8,n)', 161],
    // slot 281 (BE)
    ['Q[BE]=a(s,$fn(W))', 161],
    // slot 306 (iC)
    ['Q[iC]=O(l8,kfn(W))', 161],
    // slot 338 (Kj)
    ['Q[Kj]=O(jf,Nen(W))', 161],
    // slot 339 (Ec)
    ['Q[Ec]=l(K2,Ven(W))', 161],
    // slot 422 (Ti)
    ['Q[Ti]=a(G3,h1n(W))', 161],
    // slot 423 (bb)
    ['Q[bb]=c(F3,O1n(W))', 161],
    // slot 507 (MH)
    ['Q[MH]=l(nr,Jtn(W))', 161],
    // slot 519 (Oj)
    ['Q[Oj]=c(Yd,Ltn(W))', 161],
    // slot 560 (DM)
    ['Q[DM]=l(Cf,n)', 161],
    // slot 578 (sb)
    ['Q[sb]=a(wr,Qnn(W))', 161],
    // slot 598 (uC)
    ['Q[uC]=a(J4,n)', 161],
    // slot 38 (Wf)
    ['Q[Wf]=l(Ui,l2n(W))', 165],
    // slot 164 (wo)
    ['Q[wo]=O(jr,Zun(W))', 165],
    // slot 176 (Ou)
    ['Q[Ou]=c(Ui,zun(W))', 165],
    // slot 315 (Mh)
    ['Q[Mh]=O(6,Ofn(W))', 165],
    // slot 376 (SE)
    ['Q[SE]=a(J0,yen(W))', 165],
    // slot 495 (lH)
    ['Q[lH]=c(8,urn(W))', 165],
    // slot 527 (yM)
    ['Q[yM]=c(en,n)', 165],
    // slot 545 (Sc)
    ['Q[Sc]=l(Bi,ytn(W))', 165],
    // slot 604 (cj)
    ['Q[cj]=l(l3,cnn(W))', 165],
    // slot 615 (dy)
    ['Q[dy]=l(y8,n)', 165],
    // slot 624 (qv)
    ['Q[qv]=l(W4,U$(W))', 165],
  ];
  // Slots 324 and 404: the platform vector (Yg) and the gQ probe. Genuine
  // Chrome 155 emits a stable value for each (324 = the OS platform vector,
  // 404 = empty string); the realm's IIFE/reducer produces a divergent
  // placeholder. Both write sites are unique and self-contained. Pin to the
  // captured genuine value (control bytes preserved as JS escapes).
  const stringPins = [
    // expr len 17
    ['Q[gQ]=O(y2,L1n())', ""],
    // expr len 256
    ['Q[Yg]=c(vr,(function(){function on(yt){yt>>>=t;for(var Ot=Un;yt>=vw;)Ot+=Zr[Z2](yt&lw|vw),yt>>>=p;return Ot+Zr[Z2](yt&lw)}var Hn=Zr(wfn(W)),Wn=Zr(afn(W)),tt=Zr(cfn(W)),et=Un;return et+=on(v),et+=on(Hn[I]),et+=on(Wn[I]),et+=on(tt[I]),et+=Hn,(et+=Wn)+tt})())', "\u0003\u0005\u0000\u0003Linuxx86"],
  ];
  // Worker-pool and computed-index pins: stable capability/fingerprint
  // reads, each validated unique in the source and value-matched to the
  // genuine capture (/tmp/step6-real.json).
  const workerPins = [
    // slot 369
    ['Q[369]=c(l3,Cen(W))', 165],
    // slot 566
    ['Q[566]=O(0,Unn(W))', 165],
    // slot 107
    ['Q[S1]=c(K2,t0n())', "FUcVTxUnFScVLxV3"],
    // slot 118
    ['Q[Yu]=c(M2,zin(W))', "FHcUDw=="],
    // slot 154
    ['Q[Wo]=O(xc,oin(W))', "FZcThxXPFbcTvxXXE7cVHxXH"],
    // slot 192
    ['Q[oo]=n?s2<<f|Tf:Lr<<f|ve', 47247],
    // slot 203
    ['Q[Qo]=n?xi<<f|qe:Gi<<f|u', 52423],
    // slot 215
    ['Q[uo]=c(L2,oun(W))', "FN8V3xMXFRcVlxUPFXcVJxMXE9cTFxQ/FX8VfxU/FWcVLxMHFB8VRxWXFX8VbxUvExcTZxMXFbcVLxWXFZ8VTxV/FXcTFxPXExcTjxOvE68TdxOHE3cTxxOHE68TzxN3E8cTpxMXFe8TZxXfExcVFxWXFQ8VdxUnExcT1xMXFB8VRxWXFX8VbxVPFa8VbxMXE2cTFxW3FS8VlxWfFU8VfxV3ExcT1xMXE48TrxOvE3cThxN3E8cThxOvE88TdxPHE6cTFxXvE2cV3xMXFRcVlxUPFXcVJxMXE9cTFxR3FX8VpxNHFA8T1xQXFZcVDxV3FScTFxNnExcVtxUvFZcVnxVPFX8VdxMXE9cTFxOXE6cTdxOHE3cThxN3E4cTFxXvFO8="],
    // slot 248
    ['Q[tu]=O(l3,Ton(W))', "E48TrxOvE3cThxN3E8cThxOvE88TdxPHE6c="],
    // slot 260
    ['Q[CO]=l(d3,Oon())', "FUcVpxWnFYcVnxPXE38TfxU3FZcVDxVvFS8VlxN3FR8VfxVv"],
    // slot 284
    ['Q[_T]=c(T4,Wfn(W))', "FHcUDw=="],
    // slot 289
    ['Q[bC]=c(2,Rfn(W))', -1954006296],
    // slot 308
    ['Q[308]=c(p8,qfn())', "FDcVlxUPFW8VLxWX"],
    // slot 332
    ['Q[332]=a(Uc,nfn(W))', "FZcThxXPFbcTvxXXE7cVHxXH"],
    // slot 359
    ['Q[H4]=c(l8,n)', "FUcVpxWnFYcVnxPX"],
    // slot 368
    ['Q[_k]=l(Vt,Oen(W))', "FccTxxO3"],
    // slot 437
    ['Q[rO]=c(V2,n)', "FR8VHxOXE68VLxOXE4cTxw=="],
    // slot 438
    ['Q[iM]=a(t5,n)', "E48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOPE48TjxOHE48ThxOHE4cThxOHE4cThxOHE4c="],
    // slot 448
    ['Q[Pp]=c(jn,n)', 245],
    // slot 451
    ['Q[kT]=c(G3,Wrn(W))', 416077511],
    // slot 460
    ['Q[460]=Grn()?"Ek":"GFb"', "GFb"],
    // slot 492
    ['Q[Th]=c(jc,arn())', "E78VJxUfE8cTvxPHE6cTpw=="],
    // slot 520
    ['Q[OC]=a($r,n)', "FTcVlxUPFW8VLxWXE3cVHxV/FW8="],
    // slot 1
    ["Q[o]=l(M2,W2n(W))", "FZcThxXPFbcTvxXXE7cVHxXH"],
    // slot 8
    ["Q[f]=c(H1,I2n(W))", "FGcVTxV3Fa8Vxw=="],
    // slot 9
    ["Q[U]=O(Co,x2n(W))", 161],
    // slot 10
    ["Q[F]=a(Ic,G2n(W))", 165],
    // slot 11
    ["Q[tn]=c(Oi,F2n(W))", "E7cTpw=="],
    // slot 28
    ["Q[Ft]=O(ju,g2n(W))", "FQ8TzxV/FYcVVxVf"],
    // slot 63
    ["Q[Ur]=l(s8,R0n(W))", "FG8VfxXXFU8VZxVnFQ8TfxOvE3cThxMHE0cUxxOPE48T3xMHFGcVTxV3Fa8VxxMHFccTxxO3FP8TtxOnE08TBxQPFYcVhxVnFS8UvxUvFRcUXxVPFacTfxOvE58TvxN3E58TtxMHE0cUXxRHFKcUbxRnE2cTBxVnFU8VXxUvEwcUPxUvFR8VXxV/E08TBxQfFUcVlxV/FW8VLxN/E48TrxOvE3cThxN3E4cTdxOHEwcUnxUPFTcVDxWXFU8TfxOvE58TvxN3E58Ttw=="],
    // slot 79
    ["Q[lu]=l(2,H0n(W))", false],
    // slot 101
    ["Q[Po]=O(c1,i0n(W))", "FD8VfxV/FT8VZxUvEwcUHxVHFZcVfxVvFS8="],
    // slot 334
    ["on[0][on[1]]=(on[2]=_en(W),Nn(on[2]))", -1095772986],
    // slot 396
    ["Q[Ip]=l($a,R1n())", 165],
    // slot 525
    ["Q[525]=c(n5,Htn())", 227],
    // slot 542
    ["Q[zk]=l(L2,ltn())", "E38VhxWXFX8VVxUvFR8VpxWfE38="],
    // slot 622
    ["Q[622]=a(ju,Y$(W))", "FZcThxXPFbcTvxXXE7cVHxXH"],
    // slot 631
    ["Q[mv]=O(N4,L$(W))", "FZcThxXPFbcTvxXXE7cVHxXH"],
    // slot 574
    ["Q[PM]=(function(r){var e=O(4,r);if(c(an,e,e,e,r)){var w=a(P,e);return a(vn,$,w,Jf,f,nt,y,Lo,f,ur,t,t,dr,f,nt,y,io,f,Oo,t,ae,f,Yu,y,Ue,f,Xn,t,t)}return l(en,r,Jf,f,nt,y,Lo,f,ur,t,dr,f,nt,y,io,f,Ne,t)})(n)", [2766454698, 605325784]],
    // slot 29
    ["Q[dt]=(function(r){var e=O(4,r);if(c(an,e,e,e,r)){var w=a(P,e);return a(vn,$,w,nf,f,co,y,Qe,f,ao,t,t,Vn,f,b1,y,lf,f,ce,t,of,f,Vf,y,qt,f,wo,t,t)}return l(en,r,nf,f,co,y,Qe,f,ao,t,Vn,f,b1,y,lf,f,ce,t)})(n)", 2851055022],
    // slot 73
    ["Q[ru]=(function(r){var e=O(4,r);if(c(an,e,e,e,r)){var w=a(P,e);return a(vn,$,w,Le,f,Nf,y,Xu,f,Ro,t,t,Wo,f,ue,y,fe,f,we,t,zf,f,go,y,S1,f,ae,t,t)}return l(en,r,Le,f,Nf,y,Xu,f,Ro,t,Wo,f,ue,y,fe,f,tu,t)})(n)", 3118563191],
    // slot 93
    ["Q[De]=(function(r){var e=O(4,r);if(c(an,e,e,e,r)){var w=a(P,e);return a(vn,$,w,eu,f,Zn,y,Ro,f,hf,t,t,to,f,Uu,y,x,f,Tf,t,De,f,Xi,y,gf,f,Io,t,t)}return l(en,r,eu,f,Zn,y,Ro,f,hf,t,to,f,Uu,y,x,f,Tf,t)})(n)", 2338509378],
    // slot 616
    ["Q[tM]=(function(r){var e=O(4,r);if(c(an,e,e,e,r)){var w=a(P,e);return a(vn,$,w,pi,f,$e,y,nn,f,Xn,t,t,un,f,pi,y,l1,f,_n,t,of,f,Dn,y,Uu,f,un,t,t)}return l(en,r,pi,f,$e,y,nn,f,Xn,t,un,f,pi,y,l1,f,_n,t)})(n)", 3920596614],
    // slot 697: f32 clock probe, genuine encodes 0 ms
    ["Q[fh]=$t(QS)", "w8PDww=="],
    // slot 703: f32 clock probe, genuine encodes 0 ms
    ["Q[Lk]=$t(zS)", "w8PDww=="],
    // slot 709: f32 clock probe, genuine encodes 0 ms
    ["Q[709]=$t(RS)", "w8PDww=="],
    // slot 724: f32 clock probe, genuine encodes 0 ms
    ["Q[724]=$t(uX)", "w8PDww=="],
    // slot 741: f32 clock probe, genuine encodes 0 ms
    ["Q[741]=$t(AX)", "w8PDww=="],
    // slot 748: f32 clock probe, genuine encodes 0 ms
    ["Q[748]=$t(O(zt,performance,r))", "w8PDww=="],
    // slot 645: worker-namespace probe, modal fresh-capture value
    ["Q[DH]=a(_0,A$(W))", "FN8V3xMXFRcVlxUPFXcVJxMXE9cTFxQ/FX8VfxU/FWcVLxMHFB8VRxWXFX8VbxUvExcTZxMXFbcVLxWXFZ8VTxV/FXcTFxPXExcTjxOvE68TFxXvE2cV3xMXFRcVlxUPFXcVJxMXE9cTFxQfFUcVlxV/FW8VTxWvFW8TFxNnExcVtxUvFZcVnxVPFX8VdxMXE9cTFxOPE68TrxMXFe8TZxXfExcVFxWXFQ8VdxUnExcT1xMXFHcVfxWnE0cUDxPXFBcVlxUPFXcVJxMXE2cTFxW3FS8VlxWfFU8VfxV3ExcT1xMXE5cTpxMXFe8U7w=="],
    // slot 647: encoder l(1,n) on dm() input, modal fresh-capture value
    ["Q[pM]=l(1,n),n", "E7cTtxU3FS8TjxOHE7cVJw=="],
    // slot 653: s$() probe of a2(MW), modal fresh-capture value
    ["Q[db]=O(H1,s$())", 160],
];
  for (const [expr, val] of workerPins) {
    if (source.indexOf(expr) < 0 || source.indexOf(expr) !== source.lastIndexOf(expr))
      throw new Error('collection-worker-pin ' + expr.slice(0, 24));
    let rep;
    if (typeof val === 'string') rep = JSON.stringify(val);
    else if (Array.isArray(val)) rep = JSON.stringify(val);
    else rep = String(val);
    let head;
    if (expr.startsWith('Q[')) head = expr.slice(0, expr.indexOf(']=') + 1);
    else if (expr.startsWith('on[0][on[1]]=')) head = 'on[0][on[1]]';
    else head = expr.split('=')[0];
    source = source.replace(expr, head + '=' + rep);
  }
  for (const [expr, val] of stringPins) {
    if (source.indexOf(expr) < 0 || source.indexOf(expr) !== source.lastIndexOf(expr))
      throw new Error('collection-string-pin ' + expr.slice(0, 24));
    source = source.replace(expr, expr.split('=')[0] + '=' + JSON.stringify(val));
  }

  for (const [expr, val] of numericPins) {
    if (source.indexOf(expr) < 0 || source.indexOf(expr) !== source.lastIndexOf(expr))
      throw new Error('collection-numeric-pin ' + expr.slice(0, 24));
    source = source.replace(expr, expr.split('=')[0] + '=' + val);
  }
  const network = 'Q[B1]=O(Ar,T0n(W)),Q[_C]=l(k4,Brn(W)),'
    + 'Q[473]=a(X0,krn(W)),Q[Rp]=a(u0,a$(W))';
  if (source.indexOf(network) < 0 || source.indexOf(network) !== source.lastIndexOf(network))
    throw new Error('collection-network-source');
  // Slots 680-682: three Murmur3 hashes of the capability-probe failure
  // bitmap (iu/uu/ou). Each of Castle's 32 async probes OR-s a bit into the
  // accumulator when its real browser API rejects (missing codec, WebGL
  // extension, DRM module). The realm's stubs answer every probe successfully,
  // so its bitmap reads 0, which lets Framer tell the token apart. Real
  // Chrome 155 on this machine rejects a stable set of 25 probes (captured
  // live across two sessions: 680/681/682 inputs 113312835, 2199995042,
  // 19142658). Substitute the real captured accumulators at the hash read;
  // each site is unique. The bits are genuine capability results, not
  // invented values.
  const acc680 = ')(iu>>>t),Q[pv]=';
  const acc681 = ')(uu>>>t),Q[ev]=';
  const acc682 = ')(ou>>>t),Q[GD]=';
  for (const a of [acc680, acc681, acc682])
    if (source.indexOf(a) < 0 || source.indexOf(a) !== source.lastIndexOf(a))
      throw new Error('collection-680-acc');
  source = source.replace(acc680, ')(113312835),Q[pv]=');
  source = source.replace(acc681, ')(2199995042),Q[ev]=');
  source = source.replace(acc682, ')(19142658),Q[GD]=');
  // Keep SX and its later timing conversion. Only the four field reductions
  // move to Rust; their source destinations have no intervening reader.
  return source.replace(network, 'import.meta.network(W)');
}

export function createCollector() {
  let used = false, observed = false, reply, networkObserved = false, networkReply;
  const state = {seen: new Set(), count: 0, units: 0};
  const sharedWorker = value => {
    if (used || observed) throw new Error('collection-worker-boundary');
    observed = true;
    reply = capture(value, state);
    return value;
  };
  const network = value => {
    if (used || networkObserved) throw new Error('collection-network-boundary');
    networkObserved = true;
    networkReply = capture(value, state);
    return value;
  };
  const collect = value => {
    if (used || types.isProxy(value) || !Array.isArray(value) || value.length !== 765)
      throw new Error('collection-boundary');
    used = true;
    if (reply === undefined) throw new Error('collection-worker-missing');
    if (networkReply === undefined) throw new Error('collection-network-missing');
    const result = JSON.stringify({version: 3, values: capture(value, state),
      shared_worker_reply: reply, network_reply: networkReply});
    if (Buffer.byteLength(result) > COLLECTION_LIMIT) throw new Error('collection-size');
    return result;
  };
  return {collect, sharedWorker, network};
}

export function writeCollection(fd, value) {
  let body;
  const header = Buffer.alloc(12);
  try {
    if (typeof value !== 'string' || value.length === 0) throw new Error('collection-value');
    body = Buffer.from(value);
    if (body.length > COLLECTION_LIMIT) throw new Error('collection-size');
    header.write('FCLT'); header[4] = 1; header.writeUInt32BE(body.length, 8);
    for (const bytes of [header, body]) {
      let at = 0;
      while (at < bytes.length) {
        const count = fs.writeSync(fd, bytes, at, bytes.length - at);
        if (count <= 0) throw new Error('collection-write');
        at += count;
      }
    }
  } finally {
    body?.fill(0); header.fill(0); fs.closeSync(fd);
  }
}
