([jobId, offset, limit]) => {
  const job = window.__aiJobs?.[jobId];
  if (!job?.done || job.value?.status !== 200)
    throw new Error('completed successful fixture job required');
  if (![32768, 131072].includes(limit) || !Number.isSafeInteger(offset)
      || offset < 0 || offset !== (job.transferOffset ?? 0))
    throw new Error('invalid fixture transfer offset or size');
  const body = job.value.body;
  let end = Math.min(body.length, offset + limit);
  if (end < body.length) {
    const high = body.charCodeAt(end - 1), low = body.charCodeAt(end);
    if (high >= 0xD800 && high <= 0xDBFF && low >= 0xDC00 && low <= 0xDFFF)
      end -= 1;
  }
  const result = {status: job.value.status, usage: job.value.usage,
    offset, next: end, done: end === body.length, body: body.slice(offset, end)};
  if (result.done) {
    clearTimeout(job.timer);
    delete window.__aiJobs[jobId];
  } else {
    job.transferOffset = end;
  }
  return result;
}
