// Capture a real interaction trace in the user's own framer.com tab.
// Records only event shapes and timing: type, coordinates, key code names,
// button, and relative timestamps. No key values' text, no input content,
// no page data. Runs read-only alongside the user's normal activity.
(() => {
  const rec = [];
  const t0 = performance.now();
  const TYPES = ['mousemove','mousedown','mouseup','click','dblclick',
    'keydown','keyup','input','beforeinput','focusin','focusout',
    'scroll','wheel','visibilitychange','mouseover','mouseout',
    'mouseenter','mouseleave','contextmenu','paste','copy','cut',
    'touchstart','touchmove','touchend','touchcancel','select'];
  function shape(e) {
    const o = { type: e.type, dt: Math.round((e.timeStamp - t0) * 1000) / 1000 };
    if ('clientX' in e) { o.clientX = e.clientX; o.clientY = e.clientY; }
    if ('movementX' in e) { o.movementX = e.movementX; o.movementY = e.movementY; }
    if (e.type.startsWith('key')) { o.code = e.code; o.key = (e.key||'').length===1?'<char>':e.key; }
    if ('button' in e) o.button = e.button;
    if ('deltaY' in e) { o.deltaY = e.deltaY; o.deltaX = e.deltaX; }
    if (e.type.startsWith('touch') && e.touches && e.touches[0]) {
      o.clientX = Math.round(e.touches[0].clientX); o.clientY = Math.round(e.touches[0].clientY);
    }
    return o;
  }
  window.__traceStop = () => { TYPES.forEach(t => window.removeEventListener(t, h, true)); return rec; };
  const h = e => { if (rec.length < 4000) rec.push(shape(e)); };
  TYPES.forEach(t => window.addEventListener(t, h, true));
  return { armed: TYPES.length, startTime: t0 };
})()
