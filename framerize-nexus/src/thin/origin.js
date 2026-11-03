export const INVALID_UPSTREAM = 'invalid upstream origin';

const SHAPE = /^https?:\/\/[^/\\@?#\s\u0000-\u001f\u007f]+(?:\/v1)?\/?$/i;
const PATHS = new Set(['/', '/v1', '/v1/']);

function fail() {
  throw new Error(INVALID_UPSTREAM);
}

export function normalizeUpstream(value) {
  if (typeof value !== 'string' || !SHAPE.test(value)) fail();
  let url;
  try {
    url = new URL(value);
  } catch {
    fail();
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') fail();
  if (url.username || url.password || url.search || url.hash) fail();
  if (!PATHS.has(url.pathname)) fail();
  return url.origin;
}
