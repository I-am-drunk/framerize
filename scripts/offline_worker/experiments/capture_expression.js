// Read-only capture of the exact values the Castle SDK reads, evaluated in
// a blank tab of the user's real Chrome. No cookies, no storage values,
// no tokens. Output is a plain JSON object.
(async () => {
  const out = {};
  const nav = {};
  for (const k of ['appVersion','buildID','cookieEnabled','deviceMemory',
    'hardwareConcurrency','language','maxTouchPoints','oscpu','platform',
    'productSub','standalone','userAgent','vendor','webdriver']) {
    try { nav[k] = navigator[k] === undefined ? null : navigator[k]; }
    catch { nav[k] = null; }
  }
  try { nav.languages = [...navigator.languages]; } catch { nav.languages = null; }
  try { nav.javaEnabled = navigator.javaEnabled(); } catch { nav.javaEnabled = null; }
  try { nav.userAgentData = navigator.userAgentData ? {
    brands: navigator.userAgentData.brands,
    mobile: navigator.userAgentData.mobile,
    platform: navigator.userAgentData.platform,
  } : null; } catch { nav.userAgentData = null; }
  try { nav.pluginsLength = navigator.plugins.length; } catch { nav.pluginsLength = null; }
  try { nav.mimeTypesLength = navigator.mimeTypes.length; } catch { nav.mimeTypesLength = null; }
  out.navigator = nav;
  const scr = {};
  for (const k of ['width','height','availWidth','availHeight','availLeft',
    'availTop','colorDepth','pixelDepth']) {
    try { scr[k] = screen[k]; } catch { scr[k] = null; }
  }
  try { scr.orientation = screen.orientation ? screen.orientation.type : null; }
  catch { scr.orientation = null; }
  out.screen = scr;
  out.window = {
    devicePixelRatio, innerWidth, innerHeight, outerWidth, outerHeight,
    screenX, screenY,
  };
  out.hasUserAgentData = 'userAgentData' in navigator;
  out.hasWebdriver = 'webdriver' in navigator;
  // Canvas fingerprints: reproduce the standard probes the SDK family uses.
  const canvas2d = (() => {
    try {
      const c = document.createElement('canvas');
      c.width = 280; c.height = 60;
      const g = c.getContext('2d');
      g.textBaseline = 'top';
      g.font = "14px 'Arial'";
      g.fillStyle = '#f60';
      g.fillRect(125, 1, 62, 20);
      g.fillStyle = '#069';
      g.fillText('Cwm fjordbank glyphs vext quiz, 😃', 2, 15);
      g.fillStyle = 'rgba(102, 204, 0, 0.7)';
      g.fillText('Cwm fjordbank glyphs vext quiz, 😃', 4, 17);
      return c.toDataURL();
    } catch (e) { return 'ERR:' + e.message; }
  })();
  out.canvas2d = canvas2d;
  const webgl = (() => {
    try {
      const c = document.createElement('canvas');
      const g = c.getContext('webgl') || c.getContext('experimental-webgl');
      if (!g) return 'NULL';
      const dbg = g.getExtension('WEBGL_debug_renderer_info');
      return {
        vendor: g.getParameter(g.VENDOR),
        renderer: g.getParameter(g.RENDERER),
        unmaskedVendor: dbg ? g.getParameter(dbg.UNMASKED_VENDOR_WEBGL) : null,
        unmaskedRenderer: dbg ? g.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : null,
        version: g.getParameter(g.VERSION),
        shadingLanguageVersion: g.getParameter(g.SHADING_LANGUAGE_VERSION),
        maxTextureSize: g.getParameter(g.MAX_TEXTURE_SIZE),
      };
    } catch (e) { return 'ERR:' + e.message; }
  })();
  out.webgl = webgl;
  // Font probe surface: measure a few standard test strings.
  out.fonts = (() => {
    const base = ['monospace', 'sans-serif', 'serif'];
    const test = ['Arial', 'Courier New', 'Times New Roman', 'Comic Sans MS',
      'Georgia', 'Impact', 'Verdana', 'Calibri', 'Consolas', 'Segoe UI'];
    const c = document.createElement('canvas').getContext('2d');
    const s = 'mmmmmmmmmmlli';
    const baseW = {};
    for (const b of base) { c.font = "72px " + b; baseW[b] = c.measureText(s).width; }
    const found = [];
    for (const f of test) {
      for (const b of base) {
        c.font = "72px '" + f + "'," + b;
        if (c.measureText(s).width !== baseW[b]) { found.push(f); break; }
      }
    }
    return found;
  })();
  return out;
})()
