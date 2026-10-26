jobId => {
  const job = window.__aiJobs?.[jobId];
  if (!job) return null;
  const remove = () => {
    clearTimeout(job.timer);
    delete window.__aiJobs[jobId];
  };
  if (job.done && job.value?.status === -1) {
    remove();
    return {...job.value, done: true};
  }
  const queue = job.queue;
  if (!queue || (!queue.parts.length && !job.done)) return null;
  const body = queue.parts.splice(0).join('');
  const offset = queue.delivered;
  queue.delivered += body.length;
  queue.units = 0;
  const resume = queue.resume;
  queue.resume = null;
  if (resume) resume();
  const done = job.done;
  if (done) remove();
  return {status: queue.status, usage: queue.usage, body, done,
    offset, next: queue.delivered, peak_units: queue.peakUnits,
    received_bytes: queue.received};
}
