async request => {
  const refuse = () => { throw new Error('egress observation refused'); };
  if (!request || Object.keys(request).sort().join() !== 'timeout_ms,url') refuse();
  const target = new URL(request.url);
  if (target.protocol !== 'https:' || target.username || target.password || target.hash ||
      !Number.isInteger(request.timeout_ms) || request.timeout_ms < 1 ||
      request.timeout_ms > 30000) refuse();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), request.timeout_ms);
  let response, reader;
  try {
    response = await fetch(target.href, {method: 'GET', credentials: 'omit',
      redirect: 'error', cache: 'no-store', referrerPolicy: 'no-referrer',
      mode: 'cors', signal: controller.signal});
    if (response.status !== 200 || response.redirected || response.url !== target.href ||
        response.headers.get('content-type')?.split(';')[0].trim().toLowerCase() !==
          'application/json' || !response.body) refuse();
    reader = response.body.getReader();
    const chunks = [];
    let length = 0, reads = 0;
    for (;;) {
      if (++reads > 2048) refuse();
      const {done, value} = await reader.read();
      if (done) break;
      if (!(value instanceof Uint8Array) || (length += value.byteLength) > 1024) refuse();
      if (value.byteLength) chunks.push(value.slice());
    }
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    const text = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
    const match = /^\s*\{\s*"ip"\s*:\s*"([0-9a-fA-F:.]+)"\s*\}\s*$/.exec(text);
    if (!match) refuse();
    return {version: 1, url: target.href, ip: match[1]};
  } finally {
    controller.abort();
    let cleaned = true;
    try { if (reader) await reader.cancel(); else if (response?.body) await response.body.cancel(); }
    catch { cleaned = false; }
    try { reader?.releaseLock(); } catch { cleaned = false; }
    clearTimeout(timer);
    if (!cleaned) refuse();
  }
}
