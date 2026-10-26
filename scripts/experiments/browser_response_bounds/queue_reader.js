async (response, controller, job, byteLimit) => {
  const queue = {parts: [], units: 0, peakUnits: 0, delivered: 0,
    received: 0, capacity: 131072, resume: null,
    status: response.status, usage: response.headers.get('x-framer-ai-usage')};
  job.queue = queue;
  if (!response.body) {
    if (controller.signal.aborted) throw new Error('response cancelled');
    return '';
  }
  const reader = response.body.getReader(), decoder = new TextDecoder();
  const wake = () => {
    const resume = queue.resume;
    queue.resume = null;
    if (resume) resume();
  };
  const abort = () => {
    queue.parts.length = 0;
    queue.units = 0;
    wake();
  };
  const checkAbort = () => {
    if (controller.signal.aborted) throw new Error('response cancelled');
  };
  const put = async text => {
    checkAbort();
    if (!text) return;
    if (text.length > queue.capacity) throw new Error('decoded chunk exceeds queue');
    while (queue.units + text.length > queue.capacity) {
      checkAbort();
      await new Promise(resolve => { queue.resume = resolve; });
    }
    checkAbort();
    queue.parts.push(text);
    queue.units += text.length;
    queue.peakUnits = Math.max(queue.peakUnits, queue.units);
  };
  controller.signal.addEventListener('abort', abort);
  try {
    while (true) {
      checkAbort();
      const {value, done} = await reader.read();
      checkAbort();
      if (done) {
        await put(decoder.decode());
        return '';
      }
      queue.received += value.byteLength;
      if (queue.received > byteLimit) throw new Error('response body limit exceeded');
      for (let offset = 0; offset < value.byteLength; offset += 32768) {
        checkAbort();
        await put(decoder.decode(value.subarray(offset, offset + 32768),
                                 {stream: true}));
      }
    }
  } catch (error) {
    controller.abort();
    abort();
    try { await reader.cancel(); } catch (_) {}
    throw error;
  } finally {
    controller.signal.removeEventListener('abort', abort);
    reader.releaseLock();
  }
}
