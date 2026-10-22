# Measuring the first browserless request

`scripts/measure_browserless_request.py` observes the existing reviewed client
checker. Supply that checker's exact path and SHA-256. The observer executes
the verified bytes and retains its deployment, service-generation, request,
response-framing and timeout checks. It does not deploy anything.

```sh
python3 -I -B -S scripts/measure_browserless_request.py \
  --checker /absolute/path/to/reviewed/check.py \
  --checker-sha256 REVIEWED_SHA256 --execute < /private/request.json
```

Run this only as the already-authorized single test after the reviewed release
has completed. The existing checker writes the receipt. Its `accepted` field
still describes complete response delivery, not acceptance of a memory claim.

The added `resource_observation` samples the service cgroup every 100 ms and
records memory, swap and task counts. It preserves those samples and a final
memory snapshot when the request fails. A missing initial measurement refuses
before sending. No request body, response content or credentials are recorded.

Require `resource_observation.complete` for a usable measurement. Inspect the
recorded timestamps and maximum sampling gap. `sampled_peak_bytes` is a lower
bound on the actual peak. It includes concurrent work in the shared service;
it does not isolate browserless memory or establish savings. The original
`memory.peak` counter is a lifetime high-water mark and is never reset.

`sampler_joined` describes only the observer thread. It does not establish
account cleanup. Existing application ownership and cleanup rules still apply.

Four focused checks use temporary counters, including a spike between endpoint
samples and a failed request. No live inference has been measured by them.

The observer now preserves the checker's deadline exception during final
readings. A regression sends a real alarm signal at both final-reading sites;
both fail against the original observer and pass with the correction. All five
local cases pass. This correction has not had independent source review or a
live run. The first live request will use the accepted checker directly.
