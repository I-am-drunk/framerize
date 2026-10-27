async request => {
  'use strict';
  const refuse = () => { throw new Error('browser mint observation refused'); };
  try {
    const wrapper = window.__fzResidentWrapper;
    const current = () => {
      if (location.origin !== 'https://framer.com' || location.href !== request.page_url)
        refuse();
      if (!wrapper || !Object.isFrozen(wrapper) || window.__fzResidentWrapper !== wrapper
          || wrapper.realm !== window || wrapper.url !== request.wrapper_url
          || wrapper.entry !== 'a' || typeof wrapper.callable !== 'function'
          || !wrapper.module || wrapper.module.a !== wrapper.callable)
        refuse();
      if (localStorage.getItem('__cuid') !== request.device) refuse();
      const devices = document.cookie.split(';').map(part => part.trim())
        .filter(part => part.startsWith('__cuid='))
        .map(part => part.slice('__cuid='.length));
      if (devices.length !== 1 || devices[0] !== request.device) refuse();
    };
    current();
    const response = await fetch('https://api.framer.com/web/users/me', {
      credentials: 'include', redirect: 'error', cache: 'no-store',
      headers: {'x-requested-by': 'Framer'},
    });
    if (response.status !== 200) {
      if (response.body) await response.body.cancel();
      refuse();
    }
    if (!response.body) refuse();
    const reader = response.body.getReader();
    const decoder = new TextDecoder('utf-8', {fatal: true});
    let total = 0, text = '', done = false;
    try {
      while (!done) {
        const part = await reader.read();
        done = part.done;
        if (done) break;
        total += part.value.byteLength;
        if (total > 1024 * 1024) refuse();
        text += decoder.decode(part.value, {stream: true});
      }
      text += decoder.decode();
    } finally {
      try {
        if (!done) await reader.cancel();
      } finally {
        reader.releaseLock();
      }
    }
    const profile = JSON.parse(text);
    if (!profile || typeof profile.id !== 'string' || profile.id !== request.user)
      refuse();
    current();
    const token = await Reflect.apply(wrapper.callable, wrapper.module, []);
    current();
    if (typeof token !== 'string' || token.length === 0
        || new TextEncoder().encode(token).byteLength > 16384) refuse();
    return {version: 1, token, matched: {user: true, device: true, page: true, wrapper: true}};
  } catch (_) {
    refuse();
  }
}
