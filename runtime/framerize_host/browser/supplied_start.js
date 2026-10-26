async ([request, id]) => {
  const readResponse = __READ_RESPONSE__;
  if (location.origin !== 'https://framer.com' || Date.now() >= request.expires_unix_ms)
    throw new Error('supplied request origin or freshness mismatch');
  if (localStorage.getItem('__cuid') !== request.device)
    throw new Error('supplied request device mismatch');
  const jobs = window.__aiJobs ||= {};
  if (Object.keys(jobs).length !== 0) throw new Error('supplied context is already in use');
  const job = {done: false, value: null, controller: new AbortController(), contentType: null};
  jobs[id] = job;
  job.timer = setTimeout(() => job.controller.abort(), request.timeout_ms);
  job.settled = (async () => {
    let phase = 'fetch', status = null;
    try {
      const response = await fetch('https://api.framer.com' + request.path, {
        method: 'POST', credentials: 'include', redirect: 'error', cache: 'no-store',
        headers: {'content-type': 'application/json; charset=utf-8',
          'x-requested-by': 'Framer', authorization: 'Bearer ' + request.bearer},
        body: request.wire, signal: job.controller.signal,
      });
      status = response.status;
      phase = 'body';
      job.contentType = response.headers.get('content-type');
      await readResponse(response, job.controller, job, request.response_limit);
      return {status, usage: response.headers.get('x-framer-ai-usage'), body: ''};
    } catch (_) {
      return {status: -1, usage: null, body: 'supplied browser request failed',
        failure_phase: phase, head_status: status};
    }
  })().then(value => {
    if (jobs[id] === job) {
      job.done = true;
      job.value = value;
    }
  });
  return id;
}
