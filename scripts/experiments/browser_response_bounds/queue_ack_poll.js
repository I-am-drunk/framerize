([jobId, acknowledged]) => {
  if (!Number.isSafeInteger(acknowledged) || acknowledged < -1) {
    throw new Error('invalid response acknowledgement');
  }
  const job = window.__aiJobs?.[jobId];
  if (!job) return {kind: 'missing'};
  if (!job.delivery && acknowledged !== -1) {
    throw new Error('invalid initial response acknowledgement');
  }
  const state = job.delivery || {acknowledged: -1, offset: 0, pending: null};
  if (acknowledged !== state.acknowledged &&
      acknowledged !== state.pending?.frame.sequence) {
    throw new Error('invalid response acknowledgement');
  }
  job.delivery = state;
  const queue = job.queue;
  // A pending frame owns its queue capacity until its receipt is acknowledged.
  if (state.pending && acknowledged === state.pending.frame.sequence) {
    const {frame, units} = state.pending;
    if (queue && !job.controller.signal.aborted && queue.units < units) {
      throw new Error('response queue accounting mismatch');
    }
    if (queue) queue.units = job.controller.signal.aborted ? 0 : queue.units - units;
    state.acknowledged = acknowledged;
    state.offset = frame.next;
    state.pending = null;
    if (frame.done) {
      clearTimeout(job.timer);
      delete window.__aiJobs[jobId];
      return {kind: 'closed', sequence: acknowledged};
    }
    const resume = queue?.resume;
    if (queue) queue.resume = null;
    if (resume) resume();
  }
  if (state.pending) return state.pending.frame;
  const failure = job.done && job.value?.status === -1;
  if (!failure && (!queue || (!queue.parts.length && !job.done))) {
    return {kind: 'pending'};
  }
  const body = failure ? job.value.body : queue.parts.splice(0).join('');
  const units = failure ? 0 : body.length;
  const value = failure ? {...job.value} : {
    status: queue.status, usage: queue.usage, body,
    peak_units: queue.peakUnits, received_bytes: queue.received,
  };
  const frame = Object.freeze({...value, kind: 'chunk',
    sequence: state.acknowledged + 1, done: job.done,
    offset: state.offset, next: state.offset + units});
  state.pending = {frame, units};
  return frame;
}
