// Runs inside the real Chrome tab. Wraps 2D-context reads, imports the
// pinned wrapper graph (CDN URLs intercepted to local assets), and mints
// one genuine token. Returns the recorded probe answers.
(async () => {
  const reads = [];
  const errors = [];
  const C2D = window.CanvasRenderingContext2D && window.CanvasRenderingContext2D.prototype;
  if (C2D) {
    for (const m of ['getImageData', 'measureText', 'isPointInPath', 'isPointInStroke']) {
      const orig = C2D[m];
      if (!orig) continue;
      C2D[m] = function(...args) {
        const out = orig.apply(this, args);
        try {
          let s;
          if (m === 'getImageData') s = { w: out.width, h: out.height, d: Array.from(out.data) };
          else if (m === 'measureText') s = { t: String(args[0]).slice(0, 32), w: out.width };
          else s = out;
          reads.push({ m, a: args.map(x => typeof x === 'number' ? x : String(x).slice(0, 32)), s });
        } catch {}
        return out;
      };
    }
  }
  window.castlePublishableAPIKey = 'pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB';
  window.addEventListener('error', e => errors.push(String(e.message)));
  const wrapper = await import('https://app.framerstatic.com/chunk-5LKSDTQ5.mjs');
  const mint = wrapper.a;
  let token = null, tokenLength = 0, mintError = null;
  try {
    token = await Promise.race([
      mint({ budget_ms: 8000 }),
      new Promise((_, rej) => setTimeout(() => rej(new Error('mint-timeout')), 9000)),
    ]);
    tokenLength = token ? token.length : 0;
  } catch (e) { mintError = String(e && e.message || e); }
  return {
    reads, errors, tokenLength, mintError,
    navigator: {
      userAgent: navigator.userAgent,
      platform: navigator.platform,
      vendor: navigator.vendor,
      webdriver: navigator.webdriver,
    },
  };
})()
