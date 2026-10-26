window.fixtureState = async value => {
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open("fixture", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("state");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    const cache = await caches.open("fixture");
    if (value !== null) {
      localStorage.setItem("fixture", value);
      sessionStorage.setItem("fixture", value);
      document.cookie = `fixture_session=${value}; Path=/; Secure; SameSite=Strict`;
      await cache.put("/cache-marker", new Response(value));
      await new Promise((resolve, reject) => {
        const transaction = db.transaction("state", "readwrite");
        transaction.oncomplete = resolve;
        transaction.onerror = () => reject(transaction.error);
        transaction.onabort = () => reject(transaction.error);
        transaction.objectStore("state").put(value, "marker");
      });
    }
    const stored = await new Promise((resolve, reject) => {
      const request = db.transaction("state").objectStore("state").get("marker");
      request.onsuccess = () => resolve(request.result ?? null);
      request.onerror = () => reject(request.error);
    });
    const cached = await cache.match("/cache-marker");
    return [localStorage.getItem("fixture"), sessionStorage.getItem("fixture"),
      document.cookie, stored, cached ? await cached.text() : null];
  } finally {
    db.close();
  }
};

window.beginHeld = path => {
  window.firstChunk = null;
  window.heldResult = fetch(path).then(async response => {
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    window.firstChunk = decoder.decode((await reader.read()).value);
    const tail = decoder.decode((await reader.read()).value);
    if (!(await reader.read()).done) throw new Error("unexpected third chunk");
    return window.firstChunk + tail;
  });
  window.heldResult.catch(() => {});
};
