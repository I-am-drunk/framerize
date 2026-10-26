# Shared browser context experiment

This Linux fixture compares two synthetic accounts in one browser with two
separate browsers. It is an experiment, not a serving implementation.
The local HTTPS server holds one response open while the other account sends
requests and closes. It checks cookies, localStorage, sessionStorage,
IndexedDB, CacheStorage, connection ownership and complete process cleanup.

The browser binary is launched directly through Playwright. This does not
exercise the production Camoufox configuration generator, account startup,
SDK, editor, authentication or upstream requests. TLS certificate exceptions
apply only to the fixture's ephemeral, self-signed loopback certificate.
No TLS fingerprint generation, comparison or resumption-cache test occurs.
The server negotiates HTTP/1.1 only; HTTP/2 and HTTP/3 sharing are not tested.

Memory samples include the Python harness, Playwright driver and owned
browser descendants. PSS sums only complete, stable process-tree samples.
Cgroup counters are separate accounting: they are not interchangeable with
PSS. Samples are snapshots, not continuous PSS peaks. Runs must stay within
a 2 GiB cgroup with swap disabled. All descendants must share that cgroup.

Use the pinned runtime dependencies from `runtime/requirements.txt` and an
existing compatible browser installation. The command below creates private
network and PID namespaces, mounts only executable dependencies and fixture
source, and uses a temporary browser profile. It requires bubblewrap and a
user systemd manager. Choose a fresh unit name and the shared execution lock
when running alongside other project experiments.

```bash
fixture_source=/absolute/repository/scripts/experiments/shared_context_isolation
fixture_venv=/absolute/venv
fixture_browser=/absolute/browser-directory
fixture_lock=/absolute/execution-slot.lock
fixture_mode=shared  # repeat with separate after this run exits
flock -n "$fixture_lock" systemd-run --user --wait --pipe --collect \
  --unit="framerize-context-$fixture_mode-$(date +%s)" \
  -p MemoryMax=2G -p MemorySwapMax=0 -p TasksMax=512 -p CPUQuota=200% \
  -p RuntimeMaxSec=60 -p TimeoutStopSec=5 -p NoNewPrivileges=yes -p LimitCORE=0 \
  bwrap --die-with-parent --new-session --unshare-net --unshare-pid --clearenv \
  --ro-bind /usr /usr --ro-bind /lib /lib --ro-bind /lib64 /lib64 \
  --ro-bind /etc /etc --ro-bind /sys/fs/cgroup /sys/fs/cgroup \
  --tmpfs /home --tmpfs /root --tmpfs /run --tmpfs /tmp \
  --proc /proc --dev /dev --ro-bind "$fixture_venv" "$fixture_venv" \
  --ro-bind "$fixture_browser" /browser --ro-bind "$fixture_source" /work \
  --chdir /work --setenv PATH /usr/bin:/bin --setenv LANG C.UTF-8 \
  --setenv FIXTURE_PARENT_NET "$(readlink /proc/self/ns/net)" \
  --setenv FIXTURE_PARENT_PID "$(readlink /proc/self/ns/pid)" \
  "$fixture_venv/bin/python" -I -B /work/probe.py \
  --mode "$fixture_mode" --browser /browser/camoufox
```

Success prints one JSON record after process cleanup. Preserve unsuccessful
runs too. Switching times include browser automation and a local HTTPS
round trip; they are not Framer latency, TLS handshake timing or pure account
selection timing. Storage isolation does not establish remote unlinkability.
