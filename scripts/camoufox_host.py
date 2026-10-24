#!/usr/bin/env python3
"""camoufox_host.py — framerize Camoufox (stealth Firefox) sidecar. ASYNC.

One asynchronous Playwright driver manages separate Camoufox browsers.
Each account keeps its own browser, launch configuration and storage. Device
contexts remain isolated within that browser. Account directories persist
device IDs and editor metadata; the credential store supplies session cookies.

Protocol: newline-delimited JSON on stdin/stdout (unchanged).
  -> {"id":1,"cmd":"begin","account":"<uuid>","email":"...","proxy":"http://..."?}
  -> {"id":2,"cmd":"complete","account":"<uuid>","token":"<uuid>"}
  -> {"id":3,"cmd":"mint","account":"<uuid>"}
  -> {"id":4,"cmd":"ai_post","account":"<uuid>","path":"/ai/v3/chat/","bearer":"...","body":{...}}
  -> {"id":5,"cmd":"set_cookie","account":"<uuid>","name":"session","value":"..."}
  -> {"id":6,"cmd":"eval","account":"<uuid>","expr":"<js>"}
  -> {"id":7,"cmd":"close","account":"<uuid>"}
  -> {"id":8,"cmd":"rotate","account":"<uuid>"}
  -> {"id":9,"cmd":"browser_status","account":"<uuid>"}
  -> {"id":10,"cmd":"close_idle","account":"<uuid>","host_generation":"<id>","account_incarnation":1}
  <- {"id":1,"ok":true,"result":...} / {"id":1,"ok":false,"error":"..."}

State root: $FRAMERIZE_STATE (default ~/.framerize/camoufox). Tokens: accept
v3 AND v2 (Framer ships the v2 fallback itself; Firefox mints v2).
"""
import asyncio, base64, random, uuid, json, os, shutil, sys, time
from collections import deque
from contextlib import asynccontextmanager
from pathlib import Path

LOG_DIR = os.environ.get("FRAMERIZE_LOG_DIR")
NATIVE_POINTER_LIBRARY_SHA256 = "25bdc30e0afcff4190925f27f154557643645daabf9577618af750363c326096"

def trace(msg):
    sys.stderr.write(f"[camoufox {time.strftime('%H:%M:%S')}] {msg}\n")
    sys.stderr.flush()

def emit(event, level="info", **fields):
    """Structured event — same NDJSON schema as the daemon (docs/logging.md).
    One line per event at $FRAMERIZE_LOG_DIR/events-<date>.ndjson."""
    if not LOG_DIR:
        return
    try:
        os.makedirs(LOG_DIR, exist_ok=True)
        rec = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime()),
               "level": level, "event": event, "source": "sidecar"}
        rec.update(fields)
        day = time.strftime("%Y-%m-%d", time.gmtime())
        with open(os.path.join(LOG_DIR, f"events-{day}.ndjson"), "a") as f:
            f.write(json.dumps(rec) + "\n")
    except Exception:
        pass  # logging must never break the sidecar

def browser_runtime():
    """Load browser dependencies only when an account needs a browser."""
    try:
        from camoufox.async_api import AsyncNewBrowser
        from playwright.async_api import async_playwright
    except ImportError as error:
        raise RuntimeError("camoufox not installed. Run: scripts/linux/install-camoufox.sh") from error
    if os.environ.get("FRAMERIZE_LINEAR_PYTHON_PIPE", "1") == "1":
        outcome = install()
        emit("memory.python_pipe", outcome=outcome,
             level="warn" if outcome == "unsupported" else "info")
    return AsyncNewBrowser, async_playwright

ROOT = os.environ.get("FRAMERIZE_STATE", os.path.expanduser("~/.framerize/camoufox"))
LOGIN_URL = "https://framer.com/login"
API = "https://api.framer.com"
CASTLE_CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"

# Framer serves window.castlePublishableAPIKey inline only to some UAs (never
# Firefox — verified 2026-10-02). Publishable keys are public; inject it.
# Accept v3 (preferred) AND v2 (Framer's own fallback) shaped tokens.
# The warm: exactly ONE m.a() — enough to run the v3 attempt once (~5s race,
# always fails in Firefox) and latch the wrapper's internal v3-off flag.
# After that, every later m.a() goes straight to v2 (~1s). Do NOT use the
# full MINT_JS retry loop here — that's what made warm-up cost 27-81s.
WARM_JS = """(async () => {
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
  }
  try {
    if (!window.__fzCastleM) {
      const url = "%s";
      const module = await import(url);
      window.__fzCastleM = module;
      window.__fzResidentWrapper = Object.freeze({
        module, callable: module.a, url, entry: "a", realm: window
      });
    }
    const t = await window.__fzCastleM.a();
    return typeof t === "string" && t.length > 500;
  } catch (e) { return false; }
})()""" % CASTLE_CHUNK

MINT_JS = """(async () => {
  if (!window.castlePublishableAPIKey) {
    Object.defineProperty(window, "castlePublishableAPIKey", { value: Object.freeze("pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB") });
  }
  // Import the wrapper ONCE per page and cache it. Cache-busting per retry
  // was a 5s-per-attempt mistake: each fresh module pays the wrapper's 5s v3
  // race again (v3 always loses it in Firefox, then latches off — but the
  // latch only persists within one module instance). With a cached module,
  // the first mint pays the race once; every later mint is ~1s (v2 path).
  if (!window.__fzCastleM) {
    const url = "%s";
    const module = await import(url);
    window.__fzCastleM = module;
    window.__fzResidentWrapper = Object.freeze({
      module, callable: module.a, url, entry: "a", realm: window
    });
  }
  const m = window.__fzCastleM;
  for (let i = 0; i < 3; i++) {
    try {
      const t = await m.a();
      if (typeof t === "string" && t.length > 500) return t;
    } catch (e) {}
    await new Promise(r => setTimeout(r, 1500));
  }
  // Recovery path only: one fresh module (fresh Rh latch) for transient
  // cold-load races (the original reason for cache-busting).
  try {
    const url = "%s?cb=" + Date.now();
    const fresh = await import(url);
    const t = await fresh.a();
    if (typeof t === "string" && t.length > 500) {
      window.__fzCastleM = fresh;
      window.__fzResidentWrapper = Object.freeze({
        module: fresh, callable: fresh.a, url, entry: "a", realm: window
      });
      return t;
    }
  } catch (e) {}
  return null;
})()""" % (CASTLE_CHUNK, CASTLE_CHUNK)


def post_js(path, body, bearer=None):
    """Mint inline, then POST. Used for AUTH calls (signin/continue), which are
    one-at-a-time by nature. AI calls use post_with_token_js instead so the mint
    can be gated per account -- see MINT_GAP_S."""
    auth = '"authorization": "Bearer %s",' % bearer if bearer else ""
    return """(async () => {
  const tok = await (%s);
  if (!tok) return { status: 0, usage: null, body: "castle-unavailable" };
  const body = %s;
  body.castleToken = tok;
  try {
    const r = await fetch("%s" + "%s", {
      method: "POST", credentials: "include",
      headers: { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer", %s },
      body: JSON.stringify(body),
    });
    return { status: r.status, usage: r.headers.get("x-framer-ai-usage"), body: await r.text() };
  } catch (e) { return { status: -1, usage: null, body: String(e) }; }
})()""" % (MINT_JS, json.dumps(body), API, path, auth)


AI_START_JS = """(async ([path, body, token, bearer, jobId, dev]) => {
  // The velocity/concurrency counters key on the page's __cuid (measured
  // 2026-10-05: spliced tokens claiming other devices do NOT multiply the
  // bucket). Align the ambient device with the token's claimed device BEFORE
  // firing — pooled tokens minted for device Y must not be counted against
  // whatever device the last mint left behind.
  if (dev) {
    try { localStorage.setItem('__cuid', dev); } catch(e) {}
    try { document.cookie = '__cuid=' + dev + '; domain=.framer.com; path=/; secure; samesite=None'; } catch(e) {}
  }
  // FIRE-AND-POLL (2026-10-04): the browser driver serializes long-lived
  // protocol calls per process — an evaluate that awaits a 100s fetch holds
  // EVERY other command behind it (this was the 46s-mint / phantom-collapse
  // mechanism). So the fetch starts in-page and this call returns instantly;
  // completion is polled with instant evaluates (AI_POLL_JS).
  window.__aiJobs = window.__aiJobs || {};
  const headers = { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer" };
  if (bearer) headers["authorization"] = "Bearer " + bearer;
  body.castleToken = token;
  const job = { done: false, value: null, controller: new AbortController() };
  window.__aiJobs[jobId] = job;
  // A disconnected host must not leave a fetch or its response resident.
  job.timer = setTimeout(() => {
    if (window.__aiJobs[jobId] !== job) return;
    job.controller.abort();
    delete window.__aiJobs[jobId];
  }, 900000);
  (async () => {
    try {
      const r = await fetch("%s" + path, {
        method: "POST", credentials: "include", headers, body: JSON.stringify(body),
        signal: job.controller.signal,
      });
      return { status: r.status, usage: r.headers.get("x-framer-ai-usage"), body: await r.text() };
    } catch (e) { return { status: -1, usage: null, body: String(e) }; }
  })().then(v => {
    // Cancellation deletes the slot. A late completion must not recreate it.
    if (window.__aiJobs[jobId] === job) {
      job.done = true;
      job.value = v;
    }
  });
  return jobId;
})""" % API

AI_POLL_JS = """((jobId) => {
  const j = (window.__aiJobs || {})[jobId];
  if (!j || !j.done) return null;
  clearTimeout(j.timer);
  delete window.__aiJobs[jobId];
  return j.value;
})"""

AI_CANCEL_JS = """((jobId) => {
  const jobs = window.__aiJobs || {};
  const job = jobs[jobId];
  if (!job) return false;
  delete jobs[jobId];
  clearTimeout(job.timer);
  job.controller.abort();
  return true;
})"""


def memory_snapshot(proc_root=Path("/proc"), cgroup_root=Path("/sys/fs/cgroup")):
    """Read allocation headroom, including tighter ancestor cgroup limits.

    The 90% mark stops new allocations. It never throttles the shared slice
    containing the editor or pauses work already running in a warm browser.
    """
    result = {}
    headroom = []
    try:
        mem = {}
        for line in (proc_root / "meminfo").read_text().splitlines():
            key, value = line.split(":", 1)
            if key in ("MemTotal", "MemAvailable"):
                mem[key] = int(value.split()[0]) * 1024
        result["host_available_bytes"] = mem["MemAvailable"]
        result["host_reserve_bytes"] = mem["MemTotal"] // 10
        headroom.append(mem["MemAvailable"] - result["host_reserve_bytes"])
    except (OSError, ValueError, KeyError):
        pass
    try:
        group = next(line[3:] for line in (proc_root / "self/cgroup").read_text().splitlines()
                     if line.startswith("0::"))
        node = cgroup_root / group.lstrip("/")
        while node.is_relative_to(cgroup_root):
            try:
                current = int((node / "memory.current").read_text())
                limit_text = (node / "memory.max").read_text().strip()
                if "cgroup_current_bytes" not in result:
                    result["cgroup_current_bytes"] = current
                    swap = node / "memory.swap.current"
                    if swap.exists():
                        result["cgroup_swap_bytes"] = int(swap.read_text())
                if limit_text != "max":
                    limit = int(limit_text)
                    headroom.append(limit * 9 // 10 - current)
                    result["cgroup_limit_bytes"] = min(result.get("cgroup_limit_bytes", limit), limit)
            except (OSError, ValueError):
                pass
            if node == cgroup_root:
                break
            node = node.parent
    except (OSError, StopIteration):
        pass
    if headroom:
        result["allocation_headroom_bytes"] = min(headroom)
    return result


def trim_unused_heap():
    """Return unused libc pages. Live Python objects are not collected."""
    if sys.platform != "linux":
        return None
    try:
        import ctypes
        trim = ctypes.CDLL(None).malloc_trim
        trim.argtypes = [ctypes.c_size_t]
        trim.restype = ctypes.c_int
        return bool(trim(0))
    except (AttributeError, OSError):
        return None


class MemoryPressureError(RuntimeError):
    pass


BROWSER_STARTUP_RESERVE = 1024**3


def reply_chunks(out):
    """Encode the existing NDJSON bytes without copying a large text body.

    Prepare every other field before writing, so a serialization error cannot
    leave half a JSON line in the shared pipe. The caller owns the writer lock.
    """
    result = out.get("result") if type(out) is dict else None
    body = result.get("body") if type(result) is dict else None
    if (type(body) is not str or len(body) < 65536
            or tuple(out) != ("id", "ok", "result")
            or len(result) > 16
            or any(type(key) is not str for key in result)):
        yield json.dumps(out) + "\n"
        return

    before = ['{"id": ', json.dumps(out["id"]),
              ', "ok": ', json.dumps(out["ok"]), ', "result": {']
    after = []
    parts = before
    for index, (key, value) in enumerate(result.items()):
        if index:
            parts.append(", ")
        parts.extend((json.dumps(key), ": "))
        if key == "body":
            parts.append('"')
            parts = after
            parts.append('"')
        else:
            parts.append(json.dumps(value))
    parts.append("}}\n")
    prefix, suffix = "".join(before), "".join(after)
    del before, after, parts
    yield prefix
    for start in range(0, len(body), 16384):
        # Python slices at code points. Default ASCII escaping also preserves
        # lone surrogates and matches the original encoder at chunk boundaries.
        yield json.dumps(body[start:start + 16384])[1:-1]
    yield suffix


def write_reply(stream, out):
    """Synchronous encoding adapter for offline measurements."""
    for chunk in reply_chunks(out):
        stream.write(chunk)


class AsyncReplyWriter:
    """Keep pipe backpressure off the browser loop, with one frame in flight.

    Cancellation before acquiring the lock discards the queued reply. Once a
    frame starts, its task owns the lock until the complete line is written.
    Only closing the entire writer may abandon a partially written frame.
    """

    def __init__(self, stream):
        self._fd = os.dup(stream.fileno())
        try:
            os.set_blocking(self._fd, False)
        except BaseException:
            os.close(self._fd)
            raise
        self._lock = asyncio.Lock()
        self._tasks = set()
        self._closed = False

    @property
    def busy(self):
        # A cancelled sender can leave its complete-frame task draining.
        return self._lock.locked()

    def _close_fd(self):
        if self._fd is not None:
            os.close(self._fd)
            self._fd = None

    def _finished(self, task):
        self._tasks.discard(task)
        self._lock.release()
        if not task.cancelled():
            # A cancelled caller no longer retrieves its writer's exception.
            task.exception()

    async def send(self, out):
        await self._lock.acquire()
        if self._closed:
            self._lock.release()
            raise BrokenPipeError("reply writer is closed")
        try:
            task = asyncio.create_task(self._write_frame(out))
        except BaseException:
            self._lock.release()
            raise
        self._tasks.add(task)
        task.add_done_callback(self._finished)
        await asyncio.shield(task)

    async def _write_frame(self, out):
        loop = asyncio.get_running_loop()
        fd = self._fd
        started = False
        try:
            for chunk in reply_chunks(out):
                data = memoryview(chunk.encode("ascii"))
                while data:
                    try:
                        count = os.write(fd, data)
                    except BlockingIOError:
                        ready = loop.create_future()

                        def writable():
                            if not ready.done():
                                ready.set_result(None)

                        loop.add_writer(fd, writable)
                        try:
                            await ready
                        finally:
                            loop.remove_writer(fd)
                        continue
                    if not count:
                        raise BrokenPipeError("reply pipe made no progress")
                    started = True
                    data = data[count:]
        except BaseException as exc:
            if started or isinstance(exc, (OSError, asyncio.CancelledError)):
                self._closed = True
                self._close_fd()
            raise

    async def close(self):
        self._closed = True
        tasks = tuple(self._tasks)
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._close_fd()


def prepared_memory_browser():
    """Validate an optional private browser before opening accounts."""
    configured = os.environ.get("FRAMERIZE_MEMORY_BROWSER")
    selected = _validate_memory_browser(configured) if configured else None
    if os.environ.get("FRAMERIZE_BATCH_CONSOLE_DISPOSAL") == "1":
        try:
            supports_batch = selected is not None and json.loads(
                (Path(selected).parent / "framerize-memory-patch.json").read_text()
            ).get("patch_version") in (5, 6, 7, 8, 9)
        except (OSError, ValueError, AttributeError):
            supports_batch = False
        if not supports_batch:
            os.environ["FRAMERIZE_BATCH_CONSOLE_DISPOSAL"] = "0"
            os.environ["FRAMERIZE_DISPOSE_EVICTED_CONSOLE"] = "0"
    if os.environ.get("FRAMERIZE_IDLE_CACHE_RECLAIM") == "1":
        try:
            supports_reclaim = selected is not None and json.loads(
                (Path(selected).parent / "framerize-memory-patch.json").read_text()
            ).get("patch_version") in (8, 9)
        except (OSError, ValueError, AttributeError):
            supports_reclaim = False
        if not supports_reclaim:
            os.environ["FRAMERIZE_IDLE_CACHE_RECLAIM"] = "0"
    return selected


def _validate_memory_browser(configured):
    import hashlib
    import zipfile
    try:
        directory = Path(configured).expanduser().resolve(strict=True)
        manifest = json.loads((directory / "framerize-memory-patch.json").read_text())
        version = manifest.get("patch_version")
        expected_module = {
            3: "48c7827bd61e32ea59a749c00b0a4dffe9c816dbf81754a3dae2a1c47e97a6c8",
            4: "c6c37cde0229fdd51df32604ef8cbeecb10c7b1fb0c4019483c514ca80bce142",
            5: "c6c37cde0229fdd51df32604ef8cbeecb10c7b1fb0c4019483c514ca80bce142",
            6: "f37bec3303fc06b57469afabe7759c95d20d83545fb73d86672777797c6f98d9",
            7: "f37bec3303fc06b57469afabe7759c95d20d83545fb73d86672777797c6f98d9",
            8: "f37bec3303fc06b57469afabe7759c95d20d83545fb73d86672777797c6f98d9",
            9: "f37bec3303fc06b57469afabe7759c95d20d83545fb73d86672777797c6f98d9",
        }.get(version)
        if (expected_module is None
                or manifest.get("patched_module_sha256") != expected_module
                or manifest.get("preference") != "framerize.captureResponseBodies"):
            raise ValueError("unsupported memory browser patch")
        if version in (4, 5, 6, 7, 8, 9) and manifest.get("request_preference") != "framerize.captureRequestBodies":
            raise ValueError("unsupported request capture patch")
        if version in (6, 7, 8, 9) and manifest.get("redirect_preference") != "framerize.reportActualRedirects":
            raise ValueError("unsupported finished-request tracking patch")
        executable = directory / "camoufox"
        if not executable.is_file():
            raise ValueError("memory browser executable missing")
        with (directory / "omni.ja").open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != manifest.get("patched_archive_sha256"):
            raise ValueError("memory browser archive changed")
        with zipfile.ZipFile(directory / "omni.ja") as archive:
            module = archive.read("chrome/juggler/content/NetworkObserver.js")
            if version in (5, 6, 7, 8, 9):
                batch_modules = {
                    "content/Runtime.js": "30d50f6856fd7094d8bfe6fadfe8c7ffb4dadb236692d1fdc33a9abc72e17d57",
                    "protocol/Protocol.js": "338cb8652825699a723815b9fed40ff4a70a215ecc855324add43478c6902d6f",
                    "protocol/PageHandler.js": "4845116854d77119b2eb040704206a169d12dae79526c5b515d54e8e2e0c20ec",
                    "content/PageAgent.js": "df5081a90f477a863cfad8ca4b052a650d4c9df66350a2a97f1e9a8dfe4772a7",
                    "content/WorkerMain.js": "74ae8d31377499ed699be86d662f64e5ceb045d3d35beff0ed72eb1de130e864",
                }
                if version in (6, 7, 8, 9):
                    batch_modules["protocol/Protocol.js"] = "6caf5d857e95d89b4e90fe9c885a30f3a18418e1c15b9f8a8204e1e43d1764fa"
                if version in (8, 9):
                    reclaim_module = "98931d84fc5989e98bc3dc5ce5b7341f6567557edc0070cc3f130579c764200d"
                    batch_modules["protocol/PageHandler.js"] = reclaim_module
                    if (manifest.get("graphics_reclaim_preference") != "framerize.reclaimGraphicsOnGC"
                            or manifest.get("graphics_reclaim_module_sha256") != reclaim_module
                            or manifest.get("graphics_reclaim_default_enabled") is not False):
                        raise ValueError("unsupported cache reclamation patch")
                expected_batch = {
                    "chrome/juggler/content/" + name: digest
                    for name, digest in batch_modules.items()
                }
                if manifest.get("batch_disposal_modules") != expected_batch:
                    raise ValueError("unsupported batch disposal patch")
                for name, expected in expected_batch.items():
                    if hashlib.sha256(archive.read(name)).hexdigest() != expected:
                        raise ValueError("batch disposal module changed")
            if version == 9:
                cleanup_module = "7e60cec867abcb10f3d6af75d0f3439256c03b271f9755175db90895f6f7ec44"
                if (manifest.get("failed_page_cleanup_module_sha256") != cleanup_module
                        or hashlib.sha256(archive.read(
                            "chrome/juggler/content/TargetRegistry.js")).hexdigest() != cleanup_module):
                    raise ValueError("unsupported failed-page cleanup patch")
        if hashlib.sha256(module).hexdigest() != expected_module:
            raise ValueError("memory browser module changed")
        if version in (7, 8, 9):
            if (manifest.get("native_pointer_library_sha256") != NATIVE_POINTER_LIBRARY_SHA256
                    or manifest.get("native_pointer_upstream_commit") !=
                    "da1688c62468c554f6593456cff80f3886dc21b8"):
                raise ValueError("unsupported native pointer backport")
            with (directory / "libxul.so").open("rb") as stream:
                native_digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if native_digest != NATIVE_POINTER_LIBRARY_SHA256:
                raise ValueError("native pointer library changed")
    except Exception as exc:
        emit("memory.browser_patch", level="warn", outcome="unavailable",
             error=type(exc).__name__)
        return None
    emit("memory.browser_patch", outcome="verified", patch_version=version)
    return str(executable)


MEMORY_ADDON_SOURCE_SHA256 = "a7602c21732f828c1601b47ec484b76bcecbb3b7b1459ba7ca4f429f8a3037bd"
MEMORY_ADDON_PATCHED_SHA256 = "0ca79ae890b922506e4689cadfa704ab827ee803f8b1fa8e0691adf512f0a51d"


def prepared_memory_addon():
    configured = os.environ.get("FRAMERIZE_MEMORY_ADDON")
    if not configured:
        return None
    import hashlib
    try:
        from camoufox.addons import DefaultAddons, get_addon_path
        installed = Path(get_addon_path(DefaultAddons.UBO.name)).resolve(strict=True)
        directory = Path(configured).expanduser().resolve(strict=True)
        record_name = "framerize-addon-patch.json"
        module = "js/s14e-serializer.js"
        record = json.loads((directory / record_name).read_text())
        if (directory == installed or record.get("patch_version") != 1
                or record.get("purpose") != "serializer-exception-cleanup"
                or record.get("modified_files") != [module]):
            raise ValueError("unsupported addon patch")
        before, after = record["source_files"], record["files"]
        if (before.get(module) != MEMORY_ADDON_SOURCE_SHA256
                or after.get(module) != MEMORY_ADDON_PATCHED_SHA256
                or set(before) != set(after)):
            raise ValueError("unsupported addon source")
        actual_source = {p.relative_to(installed).as_posix()
                         for p in installed.rglob("*") if p.is_file()}
        actual_candidate = {p.relative_to(directory).as_posix()
                            for p in directory.rglob("*") if p.is_file()
                            and p.relative_to(directory).as_posix() != record_name}
        if set(before) != actual_source or set(after) != actual_candidate:
            raise ValueError("addon file inventory changed")
        for name in before:
            if name != module and before[name] != after[name]:
                raise ValueError("unrelated addon change")
            for root, expected in ((installed, before[name]), (directory, after[name])):
                path = (root / name).resolve(strict=True)
                if not path.is_relative_to(root):
                    raise ValueError("addon file outside its directory")
                with path.open("rb") as stream:
                    if hashlib.file_digest(stream, "sha256").hexdigest() != expected:
                        raise ValueError("addon file changed")
    except Exception as exc:
        emit("memory.addon_patch", level="warn", outcome="unavailable",
             error=type(exc).__name__)
        return None
    emit("memory.addon_patch", outcome="verified", patch_version=1)
    return {"path": str(directory), "excluded_default": DefaultAddons.UBO}


POOL_CAP = int(os.environ.get("FRAMERIZE_PAGE_POOL", "6"))

# Offline Castle token forge (harvest once, mint locally forever after).
try:
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import castle_forge as _cf
except Exception:
    _cf = None
FORGE_MAX_AGE_S = float(os.environ.get("FRAMERIZE_FORGE_MAX_AGE", "1500"))  # re-harvest every 25min
# The workspace concurrency cap counts PER DEVICE (verified 2026-10-04: 6
# concurrent generations on __cuid A + 6 on __cuid B, same workspace/route/
# model → ALL 200; one device alone 429s past ~5). A pool of device ids, each
# with its OWN castle module instance (modules bind the device at configure
# time — re-minting with a swapped __cuid on a cached module does NOTHING,
# verified), rotates the bucket per mint. Warmed concurrently at launch.
# VELOCITY GOOSE (2026-10-05, proven live): the per-account velocity ban is
# keyed per (account x DEVICE), not per account — same account on a fresh
# __cuid got 5/5 200s while the banned device sat at 403 (all routes, both
# JWT types). N stable persisted device ids = N x velocity budget on ONE
# account, and N x the per-device concurrency cap (~20-24) for free. These
# are STABLE identities (persisted in framerize-devices.json), not
# burn-and-churn — "user owns N devices" is a real pattern.
DEVICE_POOL = int(os.environ.get("FRAMERIZE_DEVICE_POOL", "4"))  # realistic human device count (laptop/phone/desktop/tablet); >10/account is a Castle review signal
# BASELINE-TRAINING HEARTBEAT (the velocity golden goose, from Castle's own
# docs 2026-10-07): high_activity_device "learns what is typical behavior for
# a device and detects ANOMALOUS BURSTS" — the trip line is RELATIVE to the
# device's learned baseline. Idle-for-hours-then-burst is the trigger; a
# steady rhythm reads as normal at production rates. So every pool device is
# kept at a constant light-user cadence of REAL free-route requests, forever.
# Cost: 8 devices x ~7.5/hr = 60 requests/hour/account of unmetered traffic.
HEARTBEAT_S = float(os.environ.get("FRAMERIZE_HEARTBEAT_S", "60"))
# BURST-RESERVE DEVICES (user's burn-and-replace idea, scoped to emergencies
# only, 2026-10-07): the velocity counter is fingerprint-BLIND (measured: a
# fresh __cuid on a KNOWN, previously-banned fingerprint still gets the full
# ~408-request grace budget — identical to a new fingerprint). So when the
# whole stable pool is velocity-cooling, mint fresh reserve devices instead
# of parking requests. Stable pool + heartbeat stays the steady-state (trained
# devices beat grace thresholds with zero churn signature); reserves are the
# burst reserve, capped per day to bound Castle's new_device/farm heuristics
# (>10 devices/account flags — docs). Never persisted to devices.json.
RESERVE_PER_DAY = int(os.environ.get("FRAMERIZE_RESERVE_PER_DAY", "8"))
DEV_COOLDOWN_S = 600.0   # velocity-banned device sits out ~10 min (measured 2-15)
DEV_WINDOW_S = 600.0     # proactive pre-ban rotation window
DEV_WINDOW_CAP = 180     # ~180 reqs/10min/device = below the measured trip line

# Castle mints are serialized PER ACCOUNT, and paced.
#
# Why this exists (reference/analysis/framer-castle-token.md): a Castle request
# token is single-shot, minted per request, and fingerprints the browser engine
# + the stable device id (__cuid). Every page in an account's pool shares that
# one device id. So N pages minting at once look like one device firing N
# fingerprint requests simultaneously -- "repeated regeneration is exactly the
# pattern risk engines punish". Measured 2026-10-04: at 100 concurrent, 28% of
# requests came back 403 risk-refused once the 429s were handled.
#
# Retrying a refusal makes it WORSE (another mint from the same refused
# device), so prevention is the only correct layer. A mint is ~1s on the warmed
# v2 path, while a generation is 10-90s, so serializing mints costs almost
# nothing in throughput and removes the burst signature entirely.
MINT_GAP_S = float(os.environ.get("FRAMERIZE_MINT_GAP_S", "0.25"))


class Device:
    """One PERMANENT device identity, living in its own isolated browser
    context (own cookie jar, localStorage, JS realm). The Castle SDK in this
    context sees exactly ONE device for its entire life — the __cuid is seeded
    BEFORE the module loads, and is never swapped. That is what hides the
    churn the velocity/risk layers key on (2026-10-11: the old single-page
    __cuid-swap pool was visible to the SDK's storage and read as a device
    farm)."""
    __slots__ = ("id", "ctx", "page", "warm", "lock")

    def __init__(self, dev_id):
        self.id = dev_id
        self.ctx = None
        self.page = None
        self.warm = False
        self.lock = asyncio.Lock()


def verify_close_protocol():
    # These are the same acknowledged-close primitives used by the observer.
    # Browser.close() itself suppresses TargetClosedError in this revision.
    from playwright._impl import _browser, _connection
    pins = ((_browser, "5d087246d6a98943623a8d0bc848b27afb16cf6fb1e9ed11fb95ffd0296f2014"),
            (_connection, "1e323ddddbc6fe2c5739760059b6cd5d7cc7b1456547ec8ce44023e4774a9d6e"))
    for module, digest in pins:
        if hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() != digest:
            raise RuntimeError("unsupported acknowledged browser close protocol")


class BrowserCloseReceipt:
    def __init__(self, browser):
        self.browser = browser
        self.acknowledged = self.closed = False
        self.disconnected = asyncio.Event()
        browser.on("disconnected", self._disconnected)
        if not browser.is_connected():
            self._disconnected()

    def _disconnected(self, *_):
        self.disconnected.set()

    async def close(self):
        if self.closed:
            return
        if not self.acknowledged:
            if self.disconnected.is_set():
                raise RuntimeError("browser exited without a close acknowledgement")
            verify_close_protocol()
            impl = self.browser._impl_obj
            if impl._should_close_connection_on_close:
                raise RuntimeError("shared protocol connection close is unsupported")
            await impl._channel.send("close", None, {"reason": "idle account eviction"})
            self.acknowledged = True
        await self.disconnected.wait()
        self.browser.remove_listener("disconnected", self._disconnected)
        self.closed = True


class ResidentBrowserHttp200:
    """Private completed browser HTTP 200 evidence, not parsed inference success."""
    __slots__ = ("host", "account", "acc", "life", "generation", "incarnation",
                 "page", "context", "browser", "project", "launch", "owner",
                 "frame", "invalid", "listeners", "provenance")

    def __init__(self, host, account, acc, page, project):
        import uuid
        self.provenance = str(uuid.uuid4())
        self.host, self.account, self.acc = host, account, acc
        self.life, self.generation = acc.lifecycle, host.generation
        self.incarnation = self.life.incarnation
        self.page, self.context, self.browser = page, page.context, acc.browser
        self.frame = page.main_frame
        self.project, self.launch = project, acc.resident_launch
        self.owner = acc if page is acc.page else next(
            (d for d in acc.dev_objs.values() if d.page is page), None)
        self.invalid, self.listeners = False, []

    def current(self):
        try:
            return (not self.invalid and not self.acc.closing
                and self.host.generation == self.generation
                and self.host.accounts.get(self.account) is self.acc
                and self.host.lifecycles.get(self.account) is self.life
                and self.acc.lifecycle is self.life and self.life.phase == "resident"
                and self.life.incarnation == self.incarnation
                and self.life.browser is self.browser and self.acc.browser is self.browser
                and self.launch is not None and self.acc.resident_launch is self.launch
                and self.launch[0] is self.browser and self.launch[1] == self.incarnation
                and type(self.project) is str and 0 < len(self.project) <= 256
                and self.acc.project == self.project
                and self.owner is not None and self.owner.page is self.page
                and self.owner.ctx is self.context
                and (self.owner is self.acc or any(
                    d is self.owner for d in self.acc.dev_objs.values()))
                and self.page.context is self.context and self.page.main_frame is self.frame
                and self.context.browser is self.browser
                and any(p is self.page for p in self.context.pages)
                and any(c is self.context for c in self.browser.contexts)
                and not self.page.is_closed() and self.browser.is_connected())
        except Exception:
            return False

    def watch(self):
        # Playwright caches wrapper metadata on bound-method owners. This
        # slotted record cannot accept those attributes; ordinary functions
        # keep the adapter cache while listeners retains their exact identity.
        def navigated(frame):
            self.navigated(frame)

        def invalidated(*args):
            self.invalidate(*args)

        try:
            for source, event, callback in (
                    (self.page, "framenavigated", navigated),
                    (self.page, "close", invalidated),
                    (self.context, "close", invalidated),
                    (self.browser, "disconnected", invalidated)):
                self.listeners.append((source, event, callback))
                source.on(event, callback)
            return self.current()
        except Exception:
            return False

    def navigated(self, frame):
        # Also invalidates same-URL reloads; subframe navigation is unrelated.
        if frame is self.frame:
            self.invalidate()

    def invalidate(self, *_):
        self.invalid = True
        if self.acc._completed_browser_http200 is self:
            self.acc._completed_browser_http200 = None
        listeners, self.listeners = self.listeners, []
        for source, event, callback in listeners:
            try:
                source.remove_listener(event, callback)
            except Exception:
                pass


class AccountLifecycle:
    """Small same-host ledger; browser eviction must not reset device limits."""
    def __init__(self):
        from resident_activity import ActivityLedger
        self.activity = ActivityLedger()
        self.resident_uncertain = None
        self.creation_lock = asyncio.Lock()
        self.incarnation = 0
        self.phase = "absent"
        self.browser = None
        self.close_receipt = None
        self.close_task = None
        self.active_commands = 0
        self.cooling = {}
        self.requests = {}
        self.reserve_day = ""
        self.reserve_used = 0
        self.last_mint = 0.0


class Account:
    """One account's browser context + a POOL of pages.

    Pages are NOT the concurrency unit for I/O-bound generations: measured
    2026-10-04, one page serves 12 concurrent real fetches in 4.5s (vs 36s
    serial), because an awaited fetch yields the JS main thread. Pages exist to
    parallelize CPU-bound in-page work (the Castle mint) and to isolate failures.

    `mint_lock` + `last_mint` serialize and pace Castle mints across the whole
    account, since all its pages share one device id (__cuid)."""

    def __init__(self, mgr, ctx, page, lifecycle=None):
        self.lifecycle = lifecycle if lifecycle is not None else AccountLifecycle()
        self.mgr = mgr
        self.ctx = ctx
        self.page = page  # shared ambient page: I/O-bound generations share it (one page serves 12 concurrent fetches, measured)
        self.pool = asyncio.Queue()
        self.pool.put_nowait(page)
        self.borrowed = set()
        self.size = 1
        self.lock = asyncio.Lock()  # lifecycle only (page creation, close)
        self.mint_lock = asyncio.Lock()  # one Castle mint at a time per device
        self.devices = []           # persisted pool of __cuid device ids
        self.devpool_ready = False
        self.project = None         # stashed from real ai_posts (heartbeat)
        self.bearer = None
        self.reserve = set()        # in-memory burst-reserve device ids (never persisted)
        self.reserve_warmed = set() # reserve ids with a warm castle module
        self.forge_parts = {}       # device cuid -> {parts, born} for offline forging
        self.editor_url = None      # project editor this account "has open"
        self.editor_file = None
        self.token_pool = {}        # castle token pool: id -> {token, ts, in_use, uses, dev}
        self.dev_cooling = self.lifecycle.cooling
        self.dev_reqs = self.lifecycle.requests
        self.dev_rr = 0             # device round-robin cursor
        self.mint_page = None       # DEDICATED mint page — never runs fetches
        self.browser = None         # non-persistent camoufox Browser (context parent)
        self.dev_objs = {}          # device id -> Device (isolated context each)
        self.primary_dev = None     # STICKY device: all requests ride it until it cools
        self.session_cookie = None  # (name, value) — fanned into every device context
        self.last_used = time.monotonic()
        self.active_requests = 0
        self.closing = False
        self.tasks = set()
        self.last_gc = 0.0
        self.gc_page = 0
        self._completed_browser_http200 = None
        self.resident_launch = None  # Private (browser, incarnation, proxy, main_world_eval).

    @property
    def completed_browser_http200(self):
        evidence = self._completed_browser_http200
        if evidence is not None and not evidence.current():
            evidence.invalidate()
        return self._completed_browser_http200

    def clear_browser_http200(self):
        if self._completed_browser_http200 is not None:
            self._completed_browser_http200.invalidate()

    @property
    def reserve_day(self):
        return self.lifecycle.reserve_day

    @reserve_day.setter
    def reserve_day(self, value):
        self.lifecycle.reserve_day = value

    @property
    def reserve_used(self):
        return self.lifecycle.reserve_used

    @reserve_used.setter
    def reserve_used(self, value):
        self.lifecycle.reserve_used = value

    @property
    def last_mint(self):
        return self.lifecycle.last_mint

    @last_mint.setter
    def last_mint(self, value):
        self.lifecycle.last_mint = value



DEVPOOL_WARM_JS = """(async () => {
  // Build the device pool: K castle-module instances, each bound to its own
  // __cuid. Modules are singletons per URL, so each device needs its own
  // import URL — warmed CONCURRENTLY (each pays the v3 race in parallel).
  if (!window.__devmods) window.__devmods = {};
  if (!window.castlePublishableAPIKey) Object.defineProperty(window, 'castlePublishableAPIKey', { value: Object.freeze('pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB') });
  const ids = %s; // JSON array of uuids, persisted per account
  const saved = (() => { try { return localStorage.getItem('__cuid'); } catch(e) { return null; } })();
  // SEQUENTIAL, never Promise.all: a module binds the device it sees at
  // import time, and parallel swaps race — every module ends up on the LAST
  // __cuid and the pool silently collapses to one device (measured: 429s).
  for (let i = 0; i < ids.length; i++) {
    const dev = ids[i];
    if (window.__devmods[dev]) continue;
    try {
      localStorage.setItem('__cuid', dev);
      document.cookie = '__cuid=' + dev + '; domain=.framer.com; path=/; secure; samesite=None';
      window.__devmods[dev] = await import('https://app.framerstatic.com/chunk-5LKSDTQ5.mjs?dev=' + i);
    } catch(e) {}
  }
  if (saved) {
    try { localStorage.setItem('__cuid', saved); } catch(e) {}
    document.cookie = '__cuid=' + saved + '; domain=.framer.com; path=/; secure; samesite=None';
  }
  return Object.keys(window.__devmods).length;
})()"""

DEVPOOL_MINT_JS = """(async ([ids, wantDev]) => {
  if (!window.__devmods || Object.keys(window.__devmods).length === 0) return null;
  window.__devidx = ((window.__devidx || 0) + 1) % ids.length;
  const dev = (wantDev && window.__devmods[wantDev]) ? wantDev : ids[window.__devidx];
  const mod = window.__devmods[dev];
  if (!mod) return null;
  // Keep the ambient device aligned with the minting module (the AI request
  // fires right after this mint, from this page).
  try {
    localStorage.setItem('__cuid', dev);
    document.cookie = '__cuid=' + dev + '; domain=.framer.com; path=/; secure; samesite=None';
  } catch(e) {}
  for (let i = 0; i < 3; i++) {
    const t = await mod.a();
    if (t) return { t, dev };
    await new Promise(r => setTimeout(r, 600));
  }
  return null;
})"""


# ---------------------------------------------------------------------------
# HUMAN PRESENCE (the root fix for Castle risk-refusal waves, 2026-10-04).
#
# The decompiled Castle v3 SDK registers document-level listeners for
# mousedown/mouseup/click/keydown/keyup/mouseenter/mouseleave/visibilitychange
# and maintains behavioral buffers (event counters + timestamps) that feed the
# risk verdict attached to every request token. Our account pages sat IDLE
# forever: every minted token was a signed affidavit of "zero human events,
# machine-cadence mints" — precisely the signature Castle exists to catch.
# Measured waves: after sustained machine-only traffic, Castle hard-refused
# ALL accounts on the host for minutes (28ms refusals), recovering only after
# a quiet period.
#
# The fix: continuous low-rate human activity on every account page (trusted
# Playwright input events fire those listeners), a fresh burst right before
# each mint (the buffer's newest events ride the token), and non-metronomic
# timing everywhere.
# ---------------------------------------------------------------------------

SAFE_CLICK_JS = """([x, y]) => {
  const el = document.elementFromPoint(x, y);
  if (!el) return false;
  if (el === document.body || el === document.documentElement) return true;
  if (el.closest('a,button,input,select,textarea,[role="button"],form')) return false;
  return el.tagName === 'DIV' || el.tagName === 'MAIN' || el.tagName === 'SECTION';
}"""

async def human_burst(page, rng, allow_click=True, rpc=None):
    """One small human-like activity burst: a CURVED mouse path with
    ease-in-out timing, sometimes a wheel scroll, rarely a verified-safe
    click. Castle's docs name "straight vector mouse movements" and "time
    patterns between interactions" as bot tells — a single
    mouse.move(steps=N) lays N collinear points, exactly that tell. So the
    path is a quadratic bezier with smoothstep easing (humans accelerate and
    decelerate), jitter, and occasional mid-path hesitation. Each burst is
    ~200-500ms — cheap enough to keep every page's behavioral buffer warm."""
    if rpc is None:
        async def rpc(call, *args, **kwargs):
            return await call(*args, **kwargs)
    x0, y0 = rng.uniform(60, 1100), rng.uniform(60, 560)
    x1, y1 = rng.uniform(60, 1100), rng.uniform(60, 560)
    cx, cy = (x0 + x1) / 2 + rng.uniform(-220, 220), (y0 + y1) / 2 + rng.uniform(-160, 160)
    n = rng.randint(10, 22)
    hesitate_at = rng.randint(2, n - 2) if rng.random() < 0.25 else -1
    for i in range(n + 1):
        t = i / n
        te = t * t * (3 - 2 * t)  # smoothstep ease-in-out
        px = (1 - te) ** 2 * x0 + 2 * (1 - te) * te * cx + te ** 2 * x1
        py = (1 - te) ** 2 * y0 + 2 * (1 - te) * te * cy + te ** 2 * y1
        await rpc(page.mouse.move, px + rng.uniform(-1.5, 1.5), py + rng.uniform(-1.5, 1.5), steps=1)
        if i == hesitate_at:
            await rpc(page.wait_for_timeout, rng.uniform(90, 260))
        else:
            await rpc(page.wait_for_timeout, rng.uniform(4, 18))
    if rng.random() < 0.4:
        await rpc(page.mouse.wheel, 0, rng.uniform(-320, 320))
        x2, y2 = x1 + rng.uniform(-90, 90), y1 + rng.uniform(-60, 60)
        await rpc(page.mouse.move, x2, y2, steps=rng.randint(3, 8))
    if allow_click and rng.random() < 0.3:
        try:
            if await rpc(page.evaluate, SAFE_CLICK_JS, [int(x1), int(y1)]):
                await rpc(page.mouse.click, int(x1), int(y1), delay=rng.randint(25, 95))
        except Exception:
            pass

class Host:
    def __init__(self):
        os.makedirs(ROOT, exist_ok=True)
        self.accounts = {}  # account -> Account
        self.lifecycles = {}  # account -> lightweight history, never credentials
        self.generation = uuid.uuid4().hex
        self.proxies = {}
        self.proxy_revisions = {}
        self.lock = asyncio.Lock()  # brief: account-dict lifecycle ONLY
        self._playwright = None
        self._driver_start_task = None
        self._browser_runtime = None
        self._memory_browser = None
        self._memory_addon = None
        self._closed = False
        self._allocation_lock = asyncio.Lock()
        self._reserved_bytes = 0
        self._command_tasks = {}
        self._reply_writer = None
        self._last_heap_trim = None
        # Closing idle browsers adds a cold start. Keep them warm by default.
        self.idle_timeout = float(os.environ.get("FRAMERIZE_IDLE_TIMEOUT_S", "0"))
        configured_gc = os.environ.get("FRAMERIZE_IDLE_GC_S")
        # An explicit interval applies in both modes; only defaults accelerate.
        self.idle_gc_interval = float("120" if configured_gc is None else configured_gc)
        self.idle_gc_pressure_interval = float("60" if configured_gc is None else configured_gc)
        self._observed = None
        mode = os.environ.get("FRAMERIZE_SIDECAR_MODE", "unmanaged")
        if mode != "unmanaged":
            from observed_sidecar import MODE, ObservedHost
            if mode != MODE or DEVICE_POOL != 1 or RESERVE_PER_DAY != 0:
                raise RuntimeError("unsupported observed process configuration")
            self._observed = ObservedHost(self, ROOT,
                os.environ.get("FRAMERIZE_SIDECAR_INCARNATION"),
                os.environ.get("FRAMERIZE_OBSERVED_PROFILE_NONCE"))
        self.resident_application_generation = None
        self._resident_control = None
        self._resident_scripts = {}
        generation = os.environ.get("FRAMERIZE_RESIDENT_APPLICATION_GENERATION")
        if generation is not None:
            if (self._observed is not None or not generation.isascii()
                    or not generation.isdecimal() or len(generation) > 20
                    or not 0 < int(generation) < 2**64):
                raise RuntimeError("invalid resident startup binding")
            from resident_authorization import ResidentControl, observe_warm_identity
            self.resident_application_generation = int(generation)
            self._resident_control = ResidentControl(self)
            self._resident_scripts = {source: observe_warm_identity(source)
                                      for source in (WARM_JS, MINT_JS)}

    @asynccontextmanager
    async def _activity(self, account):
        if self._observed is not None:
            yield
            return
        life = self.lifecycles.setdefault(account, AccountLifecycle())
        with life.activity.activity():
            yield

    @staticmethod
    def _retain_resident_fence(life):
        # A held activity prevents resident admission without disabling the
        # ordinary account. Only a later, explicit proof may resolve it.
        if life.resident_uncertain is None:
            life.resident_uncertain = life.activity.activity()

    async def _rpc(self, account, call, *args, **kwargs):
        if self._observed is not None:
            return await call(*args, **kwargs)
        if args and type(args[0]) is str:
            source = getattr(self, '_resident_scripts', {}).get(args[0])
            if source is not None:
                args = (source, *args[1:])
        life = self.lifecycles[account]
        activity = life.activity.activity()
        async def actual():
            try:
                return await call(*args, **kwargs)
            except BaseException:
                self._retain_resident_fence(life)
                raise
            finally:
                activity.close()
        task = life.activity.track(actual)
        try:
            return await asyncio.shield(task)
        except asyncio.CancelledError:
            # The raw RPC is still owned, including late-created objects.
            self._retain_resident_fence(life)
            raise

    def _observed_scope(self, account):
        if self._observed is None:
            return None
        scope = self._observed.accounts.get(account)
        life = self.lifecycles.get(account)
        if (self._observed.closed or scope is None or scope.failure or scope.stopped.is_set()
                or life is None or life is not scope.lifecycle
                or life.incarnation != scope.lifecycle_incarnation):
            raise RuntimeError("observed account admission closed")
        return scope

    def _observed_scope_open(self, scope):
        life = self.lifecycles.setdefault(scope.account, AccountLifecycle())
        # Observed scopes retain their own RPC/cleanup owner. They cannot be
        # admitted through a future resident lease, even between commands.
        life.activity.fenced = True
        status = self._browser_status(scope.account)
        if (status["state"] != "absent" or life.active_commands
                or life.close_task is not None):
            raise RuntimeError("observed account cleanup is unconfirmed")
        scope.lifecycle = life
        scope.lifecycle_incarnation = life.incarnation

    def _observed_scope_closed(self, scope):
        life = scope.lifecycle
        if (self._observed.accounts.get(scope.account) is not scope
                or self.lifecycles.get(scope.account) is not life
                or life.incarnation != scope.lifecycle_incarnation
                or scope.failure or life.active_commands or life.close_task is not None):
            raise RuntimeError("observed lifecycle settlement is unconfirmed")
        acc = self.accounts.get(scope.account)
        if acc is not None and (acc.lifecycle is not life
                or acc._observed_scope is not scope or acc.browser is not life.browser):
            raise RuntimeError("observed account identity changed")
        if scope.browsers:
            if len(scope.browsers) != 1:
                raise RuntimeError("observed browser identity changed")
            owned = scope.browsers[0]
            if (not owned.closed or owned.browser is not life.browser
                    or life.close_receipt is not owned):
                raise RuntimeError("observed close acknowledgement is unconfirmed")
        elif (scope.launch_started or acc is not None or life.phase != "absent"
                or life.browser is not None or life.close_receipt is not None):
            raise RuntimeError("observed launch cleanup is unconfirmed")
        if acc is not None:
            acc.clear_browser_http200()
            acc.resident_launch = None
        self.accounts.pop(scope.account, None)
        life.browser = life.close_receipt = None
        life.phase = "absent"
        # Keep the same device windows, pacing and revisioned proxy route.

    def _observed_command(self, req):
        from observed_sidecar import SUPPORTED
        scope = self._observed.scope_for(req)
        if req.get("cmd") not in SUPPORTED or self._observed_scope(scope.account) is not scope:
            raise RuntimeError("unsupported observed account command")
        desired = req.get("proxy")
        if desired is not None and (not isinstance(desired, str) or not desired):
            raise ValueError("proxy must be a nonempty string or null")
        if not scope.route_bound:
            revision = self.proxy_revisions.get(scope.account, 0)
            if desired != self.proxies.get(scope.account):
                result = self._register_proxy(dict(account=scope.account,
                    expected_revision=revision, revision=revision + 1, proxy=desired))
                if result["outcome"] != "registered":
                    raise RuntimeError("observed proxy registration is unconfirmed")
                revision += 1
            scope.proxy, scope.proxy_revision = desired, revision
            scope.route_bound = True
        if desired != scope.proxy:
            raise RuntimeError("observed scope proxy changed")
        command = dict(req, proxy=scope.proxy, proxy_revision=scope.proxy_revision)
        self._adopt_proxy_route(command)
        return command

    def _check_proxy_revision(self, req):
        account = req.get("account", "")
        revision = req.get("proxy_revision", 0)
        if type(revision) is not int or not 0 <= revision <= 2**64 - 1:
            raise ValueError("invalid account proxy revision")
        current = self.proxy_revisions.get(account)
        if current is not None and revision != current:
            raise RuntimeError("stale account proxy revision")
        return revision

    def _adopt_proxy_route(self, req):
        account = req.get("account", "")
        revision = self._check_proxy_revision(req)
        desired = req.get("proxy", self.proxies.get(account))
        if desired is not None and (not isinstance(desired, str) or not desired):
            raise ValueError("proxy must be a nonempty string or null")
        if desired != self.proxies.get(account) and (account in self.proxy_revisions
                or self._browser_status(account)["state"] != "absent"):
            raise RuntimeError("account proxy change requires confirmed browser closure and registration")
        self.proxy_revisions[account] = revision
        if desired is None:
            self.proxies.pop(account, None)
        else:
            self.proxies[account] = desired

    def _register_proxy(self, req):
        account = req["account"]
        expected, revision, proxy = req["expected_revision"], req["revision"], req["proxy"]
        if (type(expected) is not int or type(revision) is not int
                or not 0 <= expected < revision <= 2**64 - 1 or revision != expected + 1):
            raise ValueError("invalid account proxy registration revision")
        if proxy is not None and (not isinstance(proxy, str) or not proxy):
            raise ValueError("proxy must be a nonempty string or null")
        def reply(outcome):
            return {"outcome": outcome, "status": self._browser_status(account),
                    "route": {"revision": self.proxy_revisions.get(account, 0),
                              "proxy": self.proxies.get(account)}}
        status = self._browser_status(account)
        if status["state"] != "absent" or status["active_commands"]:
            return reply("busy")
        current = self.proxy_revisions.get(account)
        if current == revision and self.proxies.get(account) == proxy:
            return reply("registered")
        if current is not None and current != expected:
            return reply("stale")
        self.proxy_revisions[account] = revision
        if proxy is None:
            self.proxies.pop(account, None)
        else:
            self.proxies[account] = proxy
        return reply("registered")

    @asynccontextmanager
    async def _allocation(self, estimate):
        async with self._allocation_lock:
            snapshot = memory_snapshot()
            available = snapshot.get("allocation_headroom_bytes")
            if available is not None and available < self._reserved_bytes + estimate:
                emit("memory.admission", level="warn", outcome="deferred", **snapshot)
                raise MemoryPressureError("browser allocation deferred at the 90% memory watermark; retry after memory is available")
            self._reserved_bytes += estimate
        try:
            yield
        finally:
            self._reserved_bytes -= estimate

    @staticmethod
    def _track_task(acc, coro, *, page=None, idle_loop=False):
        scope = getattr(acc, "_observed_scope", None)
        if scope is not None:
            return scope.background_task(coro)
        activity = None
        try:
            if not idle_loop:
                activity = acc.lifecycle.activity.activity()
            task = asyncio.create_task(coro)
        except BaseException:
            coro.close()
            if activity is not None:
                activity.close()
            raise
        if activity is not None:
            def completed(done):
                if done.cancelled() or done.exception() is not None:
                    if acc.lifecycle.resident_uncertain is None:
                        acc.lifecycle.resident_uncertain = activity
                        return
                activity.close()
            task.add_done_callback(completed)
        acc.tasks.add(task)
        task.add_done_callback(acc.tasks.discard)
        if page is not None:
            # A sleeping background task otherwise retains its closed page
            # until the next timer wake. Only cancel this page's own task.
            owner_task = task

            def page_closed(*_):
                if owner_task is not None:
                    owner_task.cancel()

            def detach(_=None):
                nonlocal owner_task
                # A damaged emitter may fail to remove the callback. Make
                # any residual callback inert without retaining the task.
                owner_task = None
                try:
                    page.remove_listener("close", page_closed)
                except Exception:
                    pass

            try:
                page.on("close", page_closed)
            except Exception:
                # Optional lifecycle tracking must not break live work.
                # Also remove a hook if registration only partly succeeded.
                detach()
            else:
                task.add_done_callback(detach)
                try:
                    # Covers a page closed before the listener was attached.
                    if page.is_closed():
                        page_closed()
                except Exception:
                    # Future close events still provide cleanup; retain the
                    # original polling lifetime if inspection is unavailable.
                    pass
        return task

    @asynccontextmanager
    async def _use_account(self, account, proxy=None):
        async with self._activity(account):
            async with self._use_account_active(account, proxy) as acc:
                yield acc

    @asynccontextmanager
    async def _use_account_active(self, account, proxy=None):
        life = self.lifecycles.setdefault(account, AccountLifecycle())
        life.active_commands += 1
        acc = None
        try:
            current = await self._account(account, proxy)
            if current.closing:
                raise RuntimeError("account browser is closing; retry after cleanup")
            acc = current
            acc.active_requests += 1
            acc.last_used = time.monotonic()
            yield acc
        finally:
            life.active_commands -= 1
            if acc is not None:
                acc.active_requests -= 1
                acc.last_used = time.monotonic()

    async def _dispose_account(self, acc, account):
        acc.clear_browser_http200()
        acc.resident_launch = None
        acc.closing = True
        tasks = tuple(acc.tasks)
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        # Browser closure includes every context. Other accounts keep their
        # separate browsers and continue using the shared protocol driver.
        await asyncio.wait_for(self._rpc(account, acc.lifecycle.close_receipt.close), 10)
        acc.dev_objs.clear()
        acc.token_pool.clear()
        acc.forge_parts.clear()
        acc.session_cookie = None
        acc.borrowed.clear()

    def _browser_status(self, account):
        life = self.lifecycles.get(account)
        if life is not None and life.phase == "absent" and (
                account in self.accounts or life.browser is not None or life.close_receipt is not None):
            life.phase = "unconfirmed"
        if life is not None and life.phase == "resident":
            try:
                acc = self.accounts.get(account)
                connected = (acc is not None and acc.browser is life.browser
                             and life.browser.is_connected())
            except Exception:
                connected = False
            if not connected:
                life.phase = "unconfirmed"
                if account in self.accounts:
                    self.accounts[account].closing = True
        return {"account_id": account, "host_generation": self.generation,
                "account_incarnation": 0 if life is None else life.incarnation,
                "state": ("unconfirmed" if account in self.accounts else "absent") if life is None else life.phase,
                "active_commands": 0 if life is None else life.active_commands}

    async def _close_idle(self, req):
        account = req["account"]
        current = self._browser_status(account)
        if (not isinstance(req.get("host_generation"), str)
                or type(req.get("account_incarnation")) is not int):
            raise ValueError("idle close requires the observed browser identity")
        if any(req[key] != current[key] for key in ("host_generation", "account_incarnation")):
            return {"outcome": "stale_generation", "status": current}
        if current["active_commands"] or current["state"] == "starting":
            return {"outcome": "busy", "status": current}
        if current["state"] == "absent":
            return {"outcome": "already_absent", "status": current}
        closed = await self._close_account(account)
        return {"outcome": "closed" if closed else "unconfirmed",
                "status": self._browser_status(account)}

    async def _close_account(self, account, *, idle_before=None):
        # Includes monitor-initiated closure, not just command dispatch.
        async with self._activity(account):
            return await self._close_account_active(account, idle_before=idle_before)

    async def _close_account_active(self, account, *, idle_before=None):
        if self._observed is not None:
            raise RuntimeError("observed scopes own account closure")
        life = self.lifecycles.get(account)
        if life is None:
            return account not in self.accounts
        acc = self.accounts.get(account)
        if life.active_commands or life.phase == "starting":
            return False
        if life.activity.pending and life.close_task is None:
            return False
        if acc is not None and (acc.active_requests or
                (idle_before is not None and acc.last_used >= idle_before)):
            return False
        if life.phase == "absent":
            return acc is None and life.browser is None and life.close_receipt is None
        if life.close_task is None:
            self._start_close(account, life, acc)
        return await asyncio.shield(life.close_task)

    def _start_close(self, account, life, acc):
        activity = life.activity.activity()
        life.phase = "closing"
        if acc is not None:
            acc.clear_browser_http200()
            acc.resident_launch = None
            acc.closing = True
        registered = asyncio.get_running_loop().create_future()
        async def finish():
            try:
                await registered
                return await self._finish_close(account, life, acc)
            finally:
                activity.close()
        runner = finish()
        try:
            life.close_task = asyncio.create_task(runner)
        except BaseException:
            runner.close()
            activity.close()
            life.phase = "unconfirmed"
            raise
        registered.set_result(None)
        return life.close_task

    async def _finish_close(self, account, life, acc):
        incarnation = life.incarnation
        try:
            if life.browser is None:
                raise RuntimeError("browser launch cleanup is unconfirmed")
            if life.close_receipt is None or life.close_receipt.browser is not life.browser:
                raise RuntimeError("browser close ownership is unconfirmed")
            if acc is not None:
                await self._dispose_account(acc, account)
            else:
                await asyncio.wait_for(self._rpc(account, life.close_receipt.close), 10)
            if (self.lifecycles.get(account) is not life or life.incarnation != incarnation
                    or self.accounts.get(account) is not acc):
                raise RuntimeError("browser ownership changed during cleanup")
            self.accounts.pop(account, None)
            life.browser = None
            life.close_receipt = None
            life.phase = "absent"
            return True
        except asyncio.CancelledError:
            life.phase = "unconfirmed"
            self._retain_resident_fence(life)
            raise
        except Exception as exc:
            life.phase = "unconfirmed"
            self._retain_resident_fence(life)
            emit("browser.cleanup", level="warn", outcome="unconfirmed",
                 error=type(exc).__name__)
            return False
        finally:
            life.close_task = None

    async def _monitor_once(self):
        snapshot = memory_snapshot()
        emit("memory.snapshot", accounts=len(self.accounts),
             active_requests=sum(a.active_requests for a in self.accounts.values()),
             background_tasks=sum(len(a.tasks) for a in self.accounts.values()),
             pages=sum(len(c.pages) for a in self.accounts.values() for c in a.browser.contexts),
             **snapshot)
        for account, life in list(self.lifecycles.items()):
            acc = self.accounts.get(account)
            if life.phase in ("closing", "unconfirmed") or (acc is not None and self.idle_timeout > 0):
                try:
                    cutoff = None if life.phase != "resident" else time.monotonic() - self.idle_timeout
                    await self._close_account(account, idle_before=cutoff)
                except Exception as exc:
                    emit("browser.cleanup", level="warn", outcome="error", error=type(exc).__name__)
        await self._reclaim_idle_memory(snapshot)
        self._trim_idle_heap(snapshot)

    def _trim_idle_heap(self, snapshot):
        """Release spare host heap pages only between commands under pressure."""
        headroom = snapshot.get("allocation_headroom_bytes")
        now = time.monotonic()
        if (self.idle_gc_interval <= 0 or headroom is None
                or headroom >= BROWSER_STARTUP_RESERVE
                or self._command_tasks or self._reserved_bytes
                or (self._reply_writer is not None and self._reply_writer.busy)
                or self.lock.locked() or self._allocation_lock.locked()
                or (self._last_heap_trim is not None
                    and now - self._last_heap_trim < self.idle_gc_interval)):
            return
        for acc in self.accounts.values():
            if (acc.closing or acc.active_requests or acc.borrowed
                    or acc.mint_lock.locked()
                    or any(d.lock.locked() for d in acc.dev_objs.values())
                    or any(t.get("in_use") for t in acc.token_pool.values())):
                return
        self._last_heap_trim = now
        outcome, released = "ok", False
        try:
            result = trim_unused_heap()
            if result is None:
                outcome = "unsupported"
            else:
                released = result
        except Exception:
            outcome = "unavailable"
        emit("memory.host_trim", outcome=outcome, released=released,
             duration_ms=round((time.monotonic() - now) * 1000, 3))

    async def _reclaim_idle_memory(self, snapshot):
        """Reclaim unreachable objects without unloading a warm account.

        At most three idle accounts are collected per tick, or six when the
        reserve is breached. Stop starting further work after 250 ms;
        a started collection may take longer.
        This does not clear
        cookies, storage, handles or live JS objects, and never retries a stuck
        collection in a loop. Regular requests do not wait on a host lock.
        """
        from resident_activity import ResidentRefused
        headroom = snapshot.get("allocation_headroom_bytes")
        # Start reclamation when a new browser would be refused by admission.
        # The former 256 MiB trigger left idle caches untouched while a launch
        # needed 1 GiB of headroom. Existing idle and busy checks still apply.
        if (self.idle_gc_interval <= 0 or headroom is None
                or headroom >= BROWSER_STARTUP_RESERVE):
            return
        started = time.monotonic()
        collections = 0
        pressure = headroom < 0
        cooldown = self.idle_gc_pressure_interval if pressure else self.idle_gc_interval
        collection_limit = 6 if pressure else 3
        candidates = sorted(self.accounts.items(), key=lambda item: item[1].last_gc)
        for account, acc in candidates:
            now = time.monotonic()
            if now - started >= 0.250:
                return
            if (acc.closing or acc.active_requests or now - acc.last_used < 30
                    or now - acc.last_gc < cooldown
                    or acc.mint_lock.locked() or acc.borrowed
                    or any(d.lock.locked() for d in acc.dev_objs.values())
                    or any(t.get("in_use") for t in acc.token_pool.values())):
                continue
            pages = [p for ctx in acc.browser.contexts for p in ctx.pages if not p.is_closed()]
            if not pages:
                continue
            try:
                async with self._activity(account):
                    page = pages[acc.gc_page % len(pages)]
                    acc.gc_page += 1
                    previous_gc = acc.last_gc
                    acc.last_gc = now
                    outcome = "ok"
                    try:
                        # Heartbeats also use these pages. Inspect all contexts without
                        # modifying jobs, then recheck activity after the async probe.
                        pending = await asyncio.wait_for(asyncio.gather(*(self._rpc(account, p.evaluate,
                            "Object.values(window.__aiJobs || {}).some(job => !job.done)"
                        ) for p in pages)), timeout=0.5)
                        if (any(pending) or acc.closing or acc.active_requests
                                or time.monotonic() - acc.last_used < 30
                                or acc.mint_lock.locked() or acc.borrowed
                                or any(d.lock.locked() for d in acc.dev_objs.values())
                                or any(t.get("in_use") for t in acc.token_pool.values())):
                            outcome = "busy"
                            acc.last_gc = previous_gc
                        else:
                            await asyncio.wait_for(self._rpc(account, page.request_gc), timeout=1)
                    except asyncio.CancelledError:
                        raise
                    except Exception:
                        outcome = "unavailable"
                    emit("memory.idle_gc", account=account[:8], outcome=outcome, pressure=pressure,
                         duration_ms=int((time.monotonic() - now) * 1000))
                    if outcome == "ok":
                        collections += 1
                    if outcome == "unavailable" or collections >= collection_limit:
                        return
            except ResidentRefused:
                continue

    async def _monitor(self):
        while True:
            await asyncio.sleep(30)
            await self._monitor_once()

    async def close(self):
        if self._observed is not None:
            raise RuntimeError("observed scopes own host shutdown")
        reservations = []
        try:
            # Reserve all accounts before any shutdown side effect. No await
            # occurs while admitting them and no host lock is retained.
            for life in self.lifecycles.values():
                reservations.append(life.activity.activity())
            return await self._close_host_active()
        finally:
            for activity in reservations:
                activity.close()

    async def _close_host_active(self):
        if self._observed is not None:
            raise RuntimeError("observed scopes own host shutdown")
        self._closed = True
        if self._driver_start_task is not None:
            await asyncio.shield(self._driver_start_task)
        for account in list(self.lifecycles):
            try:
                await self._close_account(account)
            except Exception:
                pass
        if self._playwright is not None:
            await self._playwright.stop()
            self._playwright = None
        self.accounts.clear()

    async def _human_loop(self, account, page):
        """Continuous human presence on one page: a small activity burst every
        ~25-90s (randomized). Dies with the page."""
        from resident_activity import ResidentRefused
        rng = random.Random()
        while True:
            await asyncio.sleep(rng.uniform(25, 90))
            try:
                if page.is_closed():
                    return
                async with self._activity(account):
                    await human_burst(page, rng, rpc=lambda call, *args, **kwargs:
                        self._rpc(account, call, *args, **kwargs))
            except ResidentRefused:
                continue
            except Exception:
                return

    async def _acquire(self, account, proxy=None):
        """Returns (page, acc, queue_wait_s). queue_wait_s is the time spent
        waiting for a free page — NOT upstream latency. Callers report it so the
        chunk sizer can subtract it: queue wait and a genuinely slow upstream
        look identical in wall-clock, and conflating them makes the sizer shrink
        chunks under load, which multiplies round trips and worsens the queue."""
        acc = await self._account(account, proxy)
        async with acc.lock:
            while not acc.pool.empty():
                page = acc.pool.get_nowait()
                if page.is_closed():
                    acc.size -= 1
                    continue
                acc.borrowed.add(page)
                return page, acc, 0.0
            if acc.size < POOL_CAP:
                async with self._allocation(64 * 1024**2):
                    page = await self._rpc(account, acc.ctx.new_page)
                    try:
                        await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
                        await self._rpc(account, page.wait_for_timeout, 800)
                    except BaseException:
                        await self._rpc(account, page.close)
                        raise
                acc.size += 1
                acc.borrowed.add(page)
                self._track_task(acc, self._human_loop(account, page), page=page, idle_loop=True)
                return page, acc, 0.0
        # At cap: this request QUEUES for a free page. Log the wait — queue
        # time dominates total latency under concurrent load (47s→190s
        # arithmetic growth was invisible until measured here).
        t0 = time.monotonic()
        page = await acc.pool.get()
        if page.is_closed():
            acc.size -= 1
            return await self._acquire(account, proxy)
        acc.borrowed.add(page)
        wait = time.monotonic() - t0
        if wait > 2.0:
            trace(f"{account[:8]}: pool wait {wait:.1f}s (all {POOL_CAP} pages busy)")
        return page, acc, wait

    async def _ensure_editor(self, acc, page, account):
        """Keep the page resident on the account's project EDITOR, not /login.

        INVISIBILITY (2026-10-05): every AI fetch carries the page's Referer
        and every Castle token embeds its page context — minting and firing
        generations from framer.com/login marked every request as "not a real
        editing session" (no logged-in user sits on the login page all day).
        A real agent user has the editor open: tokens minted there, calls
        fired from there, ambient sync/presence traffic flowing. After any
        navigation the castle module cache is gone, so re-warm it (and the
        device pool) — one-time ~6-10s per page, then fast.
        """
        if not acc.editor_url:
            return
        # 2026-10-11, measured on a FRESH profile: fires from the editor 403,
        # fires from /login 200. The residency theory holds for long-lived
        # established pages (iris), but a profile that JUST landed on the
        # editor and instantly calls AI reads as automation to Castle.
        # FRAMERIZE_NO_EDITOR=1 skips residency entirely (iMac default until
        # profiles mature).
        if os.environ.get("FRAMERIZE_NO_EDITOR") == "1":
            return
        try:
            current = page.url or ""
        except Exception:
            current = ""
        # The editor SPA redirects /projects/<id> to the slugged
        # /projects/<name>--<id> — a startswith() check on the bare-id URL
        # NEVER matched, so every ai_post re-navigated (52 in 10 min, chains
        # stalling 10-90s per chunk — "sessions not moving"). Match the id
        # anywhere in the URL instead.
        pid = acc.editor_url.rsplit('/', 1)[-1]
        if pid and pid in current:
            return
        try:
            await self._rpc(account, page.goto, acc.editor_url, wait_until="domcontentloaded", timeout=90000)
            trace(f"{account[:8]}: page -> editor ({acc.editor_url.split('/')[-1][:12]}...)")
            await self._rpc(account, page.wait_for_timeout, 1500)
            await self._rpc(account, page.evaluate, WARM_JS)
            if acc.devices:
                try:
                    await self._rpc(account, page.evaluate, DEVPOOL_WARM_JS.replace('%s', json.dumps(acc.devices)))
                except Exception:
                    pass
        except Exception as e:
            trace(f"{account[:8]}: editor navigation issue ({type(e).__name__}) — continuing")

    def _set_editor(self, acc, account, project_id):
        url = f"https://framer.com/projects/{project_id}"
        if acc.editor_url == url:
            return
        acc.editor_url = url
        if acc.editor_file:
            try:
                Path(acc.editor_file).write_text(json.dumps({"editor_url": url}))
            except Exception:
                pass

    @staticmethod
    def _release(acc, page):
        if page not in acc.borrowed:
            return
        acc.borrowed.remove(page)
        if acc.closing:
            acc.size -= 1
        else:
            # A closed page also wakes a waiter, which replaces it under the
            # creation lock. Dropping it here could strand every pool waiter.
            acc.pool.put_nowait(page)

    async def _new_mint_page(self, acc, account):
        """Create the account's dedicated mint page and warm it.

        WHY THIS PAGE EXISTS (measured 2026-10-04): a warm v2 mint costs ~25ms
        on an idle page, but mints measured p50=46s (max 166s) under load.
        Mints ran on the SAME pool pages as 100s-generation fetches, and all
        pages sit on framer.com/login — one Fission content process, one JS
        main thread, browser-serialized work. Every mint queued behind
        in-flight generations and vice versa. The "upstream collapse" was
        this contention, never the upstream (direct probes: 119 tok/s the
        same minute). A page that ONLY mints has an always-free main thread:
        mints stay ~25ms forever."""
        async with self._allocation(64 * 1024**2):
            page = await self._rpc(account, acc.ctx.new_page)
            try:
                await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
                await self._rpc(account, page.wait_for_timeout, 800)
                await self._rpc(account, page.evaluate, WARM_JS)
                if acc.devices:
                    try:
                        await self._rpc(account, page.evaluate, DEVPOOL_WARM_JS.replace('%s', json.dumps(acc.devices)))
                    except Exception:
                        pass
            except BaseException:
                await self._rpc(account, page.close)
                raise
        self._track_task(acc, self._human_loop(account, page), page=page, idle_loop=True)
        trace(f"{account[:8]}: dedicated mint page ready")
        return page

    TOKEN_TTL_S = 480.0  # 2026-10-05: TTL enforcement silently turned ON (~600s
                         # boundary measured via the velocity-cooling cascade — old
                         # tokens 403 and were misread as velocity bans). 480 = safe margin.
    TOKEN_CONTENT_MAX_AGE_S = 1800.0  # re-wrap hygiene: re-mint the base after 30min even though server TTL is unenforced
    TOKEN_MAX_USES = 5   # contract 2026-10-05: a token is a 6-USE coupon — use #7
                         # returns 403 deterministically (verified: #1-6 pass, #7+ all
                         # 403). Retire at 5 to stay under. The old per-chunk-mint
                         # design was SAFE by accident; unbounded pooling was not.
    TOKEN_POOL_CAP = 8

    async def _dev_page(self, acc, account, dev):
        """Get-or-create the device's isolated context + page, with __cuid
        seeded BEFORE the castle module loads (add_init_script runs ahead of
        any page script) and the account session cookie fanned in. The module
        is warmed once per context (the v3-race latch is per module scope)."""
        d = acc.dev_objs.get(dev)
        if d is None:
            d = Device(dev)
            acc.dev_objs[dev] = d
        # Single-flight initialization keeps simultaneous first requests from
        # overwriting and leaking each other's context.
        async with d.lock:
            return await self._prepare_device(acc, account, d)

    async def _prepare_device(self, acc, account, d):
        dev = d.id
        if d.page is not None and d.page.is_closed():
            evidence = acc._completed_browser_http200
            if evidence is not None and evidence.owner is d:
                acc.clear_browser_http200()
            if d.ctx is not None:
                await self._rpc(account, d.ctx.close)
            d.page, d.ctx, d.warm = None, None, False
        if d.page is not None and d.warm:
            return d
        async with self._allocation(64 * 1024**2):
            if d.page is None:
                ctx = await self._rpc(account, acc.browser.new_context)
                try:
                    await self._rpc(account, ctx.add_init_script,
                        f"try {{ localStorage.setItem('__cuid', {json.dumps(dev)}); }} catch(e) {{}}"
                    )
                    cookies = [{"name": "__cuid", "value": dev, "domain": ".framer.com",
                                "path": "/", "secure": True, "httpOnly": False, "sameSite": "None"}]
                    if acc.session_cookie:
                        name, value = acc.session_cookie
                        cookies.append({"name": name, "value": value, "domain": ".framer.com",
                                        "path": "/", "secure": True, "httpOnly": True, "sameSite": "None"})
                    await self._rpc(account, ctx.add_cookies, cookies)
                    page = await self._rpc(account, ctx.new_page)
                    await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=90000)
                except BaseException:
                    await self._rpc(account, ctx.close)
                    raise
                d.ctx, d.page = ctx, page
                trace(f"{account[:8]}: device {dev[:8]} context ready (isolated, pre-seeded)")
            if not d.warm:
                try:
                    await self._rpc(account, d.page.evaluate, WARM_JS)
                    d.warm = True
                except Exception as e:
                    trace(f"{account[:8]}: device {dev[:8]} warm issue ({type(e).__name__})")
        return d

    @staticmethod
    def _request_window(acc, dev, now):
        """Discard expired timestamps once, without copying the active window.

        Timestamps are appended in monotonic order on the event-loop thread.
        Use the same subtraction and strict window boundary as the previous
        filters. Do not cap the queue: every in-window request still counts.
        """
        reqs = acc.dev_reqs.get(dev)
        if reqs is None:
            return ()
        while reqs and now - reqs[0] >= DEV_WINDOW_S:
            reqs.popleft()
        return reqs

    def _pick_device_sticky(self, acc):
        """STICKY selection: the primary device serves everything until it
        actually cools; only then promote the next ready device. Server-side
        this reads as 'user opened their other laptop' — not a per-request
        rotation (the old round-robin was the loudest churn signal)."""
        now = time.monotonic()
        ready = [d for d in acc.devices
                 if acc.dev_cooling.get(d, 0) <= now
                 and len(self._request_window(acc, d, now)) < DEV_WINDOW_CAP]
        if acc.primary_dev in ready:
            return acc.primary_dev
        acc.primary_dev = ready[0] if ready else None
        return acc.primary_dev

    def _pick_device(self, acc, account=None):
        """Velocity-aware device choice: round-robin over the persisted device
        pool, skipping velocity-cooled devices and devices over the proactive
        window budget. Returns a READY device, or None when everything is
        cooling/over-budget — call _device_ready_in() for the wait time. NEVER
        fire on a cooling device: poking a banned device re-bans it for the
        full cooldown (measured 2026-10-06: the re-poke cascade kept every
        device permanently banned during the storm)."""
        devs = acc.devices or [None]
        now = time.monotonic()
        for _ in range(len(devs)):
            acc.dev_rr = (acc.dev_rr + 1) % len(devs)
            d = devs[acc.dev_rr]
            if acc.dev_cooling.get(d, 0) > now:
                continue
            reqs = self._request_window(acc, d, now)
            if len(reqs) >= DEV_WINDOW_CAP:
                continue
            return d
        # Stable pool exhausted — BURST RESERVE: a fresh __cuid carries the
        # full grace budget even on a known fingerprint (counter is
        # fingerprint-blind, measured). Capped per day to bound the
        # new_device/farm signature. Atomic: no awaits in this function.
        today = time.strftime("%Y-%m-%d", time.gmtime())
        if acc.reserve_day != today:
            acc.reserve_day, acc.reserve_used = today, 0
        if acc.reserve_used < RESERVE_PER_DAY:
            acc.reserve_used += 1
            dev = str(uuid.uuid4())
            acc.devices.append(dev)
            acc.reserve.add(dev)
            trace(f"{(account or chr(63)*8)[:8]}: burst-reserve device #{acc.reserve_used}/{RESERVE_PER_DAY} created (stable pool cooling)")
            return dev
        return None

    def _device_ready_in(self, acc):
        """Seconds until SOME device is usable: the soonest cooldown expiry, or
        the soonest window slot freeing up. None when a device is ready now."""
        now = time.monotonic()
        if not acc.devices:
            return None
        waits = []
        for d in acc.devices:
            cu = acc.dev_cooling.get(d, 0)
            if cu > now:
                waits.append(cu - now)
                continue
            reqs = self._request_window(acc, d, now)
            if len(reqs) < DEV_WINDOW_CAP:
                return None  # this device is ready now
            waits.append((reqs[0] + DEV_WINDOW_S) - now)
        return min(waits) if waits else None

    def _note_request(self, acc, dev):
        now = time.monotonic()
        if dev not in acc.dev_reqs:
            acc.dev_reqs[dev] = deque()
        self._request_window(acc, dev, now).append(now)

    def _cool_device(self, acc, dev, account):
        acc.dev_cooling[dev] = time.monotonic() + DEV_COOLDOWN_S
        # burn the device's outstanding coupons: a banned device's tokens all
        # draw from its drained velocity budget
        for tid in [k for k, v in acc.token_pool.items() if v.get("dev") == dev]:
            del acc.token_pool[tid]
        trace(f"{account[:8]}: device {str(dev)[:8]} velocity-cooling {int(DEV_COOLDOWN_S)}s (rotating)")

    @staticmethod
    def _rewrap_token(token):
        """Outer-layer re-wrap: decode the outer XOR with the embedded key byte,
        re-XOR with a fresh random byte. New token STRING, byte-identical content.
        VERIFIED 2026-10-05 (the Castle goose): the server's 6-use coupon tracker
        keys on the STRING — a burned coupon re-wrapped passes 200, and each
        re-wrapped string gets its own 6 uses. Pure python, no browser eval:
        one real mint now yields unlimited coupons."""
        raw = base64.urlsafe_b64decode(token + "=" * (-len(token) % 4))
        k1 = raw[0]
        body = bytes(b ^ k1 for b in raw[1:])
        k2 = random.randint(1, 255)
        while k2 == k1:
            k2 = random.randint(1, 255)
        return base64.urlsafe_b64encode(bytes([k2]) + bytes(b ^ k2 for b in body)).decode().rstrip("=")

    async def _token_acquire(self, acc, page, account, dev=None):
        """Castle token pool, device-aware (velocity budgets key on the token's
        embedded CUID): prefers idle coupons minted on the REQUESTED device,
        else mints on it. Sequential reuse is free; concurrent reuse of one
        token is the only deny signature (in_use flips atomically before any
        await in the reuse path — asyncio single-thread)."""
        now = time.monotonic()
        for tid, ent in list(acc.token_pool.items()):
            if now - ent.get("born", ent["ts"]) > self.TOKEN_TTL_S:
                # 2026-10-05: Framer silently enabled TTL enforcement (~600s; the
                # velocity-cooling cascade was old tokens 403ing, misread as
                # velocity bans). Retire well under the boundary.
                del acc.token_pool[tid]
                continue
            if dev is not None and ent.get("dev") != dev:
                continue  # stay on the velocity-selected device
            if ent["uses"] >= self.TOKEN_MAX_USES:
                # Coupon spent — RE-WRAP instead of re-minting: the tracker keys
                # on the token string, so a fresh string = a fresh 6-use coupon
                # for byte-identical content (verified 2026-10-05).
                ent["token"] = self._rewrap_token(ent["token"])
                ent["uses"] = 0
                ent["ts"] = now
                ent["wraps"] = ent.get("wraps", 0) + 1
                trace(f"{account[:8]}: token re-wrapped (wrap #{ent['wraps']} dev {str(ent.get('dev'))[:8]}) — fresh coupon, no mint")
            if not ent["in_use"]:
                ent["in_use"] = True
                ent["uses"] += 1
                trace(f"{account[:8]}: token reuse #{ent['uses']} dev {str(ent.get('dev'))[:8]} (age {now - ent['ts']:.0f}s, pool {len(acc.token_pool)})")
                return tid, ent["token"], ent.get("dev")
        if len(acc.token_pool) >= self.TOKEN_POOL_CAP:
            tok, used_dev = await self._mint(acc, page, account, dev)  # all busy: transient
            return None, tok, used_dev
        tok, used_dev = await self._mint(acc, page, account, dev)
        if not tok:
            return None, None, None
        tid = uuid.uuid4().hex[:12]
        acc.token_pool[tid] = {"token": tok, "ts": now, "born": now, "in_use": True, "uses": 1, "dev": used_dev}
        return tid, tok, used_dev

    def _token_release(self, acc, tid):
        if tid is None:
            return
        ent = acc.token_pool.get(tid)
        if ent:
            ent["in_use"] = False

    async def _mint(self, acc, page, account, dev=None):
        """Mint one Castle token, serialized and paced per ACCOUNT.

        All of an account's pages share one device id (__cuid), so simultaneous
        mints look like one device firing N fingerprint requests at once --
        the burst signature a risk engine punishes. The lock makes mints
        one-at-a-time and MINT_GAP_S spaces them, so the device's request
        pattern stays human-plausible no matter how many agents are running.

        This is PREVENTION. A 403 risk-refusal must never be retried: another
        mint from an already-refused device is what turns a transient refusal
        into a sticky one.
        """
        # FORGE-FIRST (2026-10-05): if we hold fresh harvested parts for the
        # requested device, mint locally — no browser eval, no pacing gap, no
        # SDK race. Parts come from any real browser mint (harvested below)
        # and are refreshed every FORGE_MAX_AGE_S.
        if _cf is not None:
            ent = None
            fdev = dev
            if dev is not None:
                ent = acc.forge_parts.get(dev)
            else:
                fresh = [(e["born"], c) for c, e in acc.forge_parts.items()]
                if fresh:
                    fdev = max(fresh)[1]
                    ent = acc.forge_parts[fdev]
            if ent and (time.monotonic() - ent["born"]) < FORGE_MAX_AGE_S:
                try:
                    tok = _cf.forge(ent["parts"], fdev)
                    emit("castle.forge", account=account[:8], device=str(fdev)[:8], outcome="ok")
                    return tok, fdev
                except Exception:
                    pass  # fall through to a live mint
        async with acc.mint_lock:
            gap = MINT_GAP_S - (time.monotonic() - acc.last_mint)
            if gap > 0:
                await asyncio.sleep(gap)
            t0 = time.monotonic()
            tok = None
            burst_ms = 0  # pre-mint burst REMOVED: it was the bottleneck
            # (19-21s under humanize=True). The per-page humanize loops keep
            # Castle's behavioral buffers warm between mints.
            used_dev = dev
            if dev is not None and acc.browser is not None:
                # CONTEXT-PER-DEVICE: mint inside the device's own isolated
                # context. The SDK binds the context's single pre-seeded
                # __cuid — no swaps, no shared-page churn, ever.
                try:
                    d = await self._dev_page(acc, account, dev)
                    tok = await self._rpc(account, d.page.evaluate, MINT_JS)
                    used_dev = dev
                except Exception as e:
                    trace(f"{account[:8]}: context mint failed on {str(dev)[:8]} ({type(e).__name__})")
                    tok = None
            if not tok:
                page = acc.mint_page
                if page is None or page.is_closed():
                    page = acc.mint_page = await self._new_mint_page(acc, account)
                tok = await self._rpc(account, page.evaluate, MINT_JS)
                used_dev = None  # single-device path (ambient __cuid)
            acc.last_mint = time.monotonic()
            if tok and _cf is not None:
                try:
                    parts = _cf.harvest(tok)
                    acc.forge_parts[parts["cuid"]] = {"parts": parts, "born": time.monotonic()}
                except Exception:
                    pass
            emit("castle.mint", account=account[:8],
                 duration_ms=int((acc.last_mint - t0) * 1000),
                 burst_ms=burst_ms, device=str(used_dev)[:8] if used_dev else None,
                 outcome="ok" if tok else "error",
                 **({} if tok else {"error": "castle-unavailable"}))
            return tok, used_dev

    async def _account(self, account, proxy=None):
        async with self._activity(account):
            return await self._account_active(account, proxy)

    async def _account_active(self, account, proxy=None):
        self._observed_scope(account)
        if self._closed:
            raise RuntimeError("browser host is closed")
        life = self.lifecycles.setdefault(account, AccountLifecycle())
        self._browser_status(account)
        if life.phase in ("closing", "unconfirmed"):
            raise RuntimeError("account browser cleanup is unconfirmed; retry after cleanup")
        if account in self.accounts and life.phase == "resident":
            return self.accounts[account]
        async with life.creation_lock:
            scope = self._observed_scope(account)
            if self._closed:
                raise RuntimeError("browser host is closed")
            if life.phase in ("closing", "unconfirmed"):
                raise RuntimeError("account browser cleanup is unconfirmed; retry after cleanup")
            if account in self.accounts:
                return self.accounts[account]
            life.incarnation += 1
            life.phase = "starting"
            if scope is not None:
                scope.lifecycle_incarnation = life.incarnation
            try:
                acc = await self._launch(account, proxy)
                if self._observed is not None:
                    try:
                        self._observed_scope(account)
                    except RuntimeError:
                        life.phase = "unconfirmed"
                        raise
            except BaseException:
                if life.phase == "starting":
                    life.phase = "absent"
                raise
            life.phase = "resident"
            return acc

    @asynccontextmanager
    async def _page(self, account, proxy=None):
        page, acc, _wait = await self._acquire(account, proxy)
        try:
            yield page
        finally:
            self._release(acc, page)

    async def _start_driver(self, factory):
        if self._playwright is not None:
            return
        if self._observed is not None:
            # Observed startup stays in its existing command owner. Other
            # scopes await only its signal, without a global launch lock.
            if self._driver_start_task is not None:
                await asyncio.shield(self._driver_start_task)
                if self._playwright is None:
                    raise RuntimeError("observed driver startup is unconfirmed")
                return
            signal = asyncio.get_running_loop().create_future()
            self._driver_start_task = signal
            try:
                self._playwright = await factory().start()
            finally:
                signal.set_result(None)
                self._driver_start_task = None
            return
        if self._driver_start_task is None:
            async def start():
                self._playwright = await factory().start()
            self._driver_start_task = asyncio.create_task(start())
        task = self._driver_start_task
        try:
            await asyncio.shield(task)
        finally:
            if task.done():
                self._driver_start_task = None

    async def _launch(self, account, proxy=None):
        self._observed_scope(account)
        if self._browser_runtime is None:
            runtime = browser_runtime()
            self._memory_browser = prepared_memory_browser()
            self._memory_addon = prepared_memory_addon()
            self._browser_runtime = runtime
        AsyncNewBrowser, async_playwright = self._browser_runtime
        if proxy:
            self.proxies[account] = proxy
        profile = os.path.join(ROOT, account)
        os.makedirs(profile, exist_ok=True)
        # CONTEXT-PER-DEVICE model (2026-10-11): a NON-persistent browser, and
        # every device gets its own isolated ephemeral context (own storage —
        # verified: ctx A cannot see ctx B's cookies). The old persistent
        # single-context profile forced the __cuid-swap churn that flagged us.
        # Browser launch timeout 30s -> 180s: under heavy machine load (the
        # iMac's chromium build pinned load avg >100), the juggler handshake
        # starves past the default 30s and every launch dies in a retry loop —
        # 180s lets it complete slowly instead of never (2026-10-11 stall).
        kwargs = dict(headless=True, humanize=False, timeout=600_000,
                      firefox_user_prefs={
                          "dom.ipc.processCount": 1,
                          "dom.ipc.processCount.webIsolated": 1,
                          "browser.cache.memory.enable": False,
                          "browser.cache.disk.enable": False,
                          "browser.sessionhistory.max_entries": 2,
                      })
        if self._memory_addon is not None:
            kwargs["addons"] = [self._memory_addon["path"]]
            kwargs["exclude_addons"] = [self._memory_addon["excluded_default"]]
        if self._memory_browser:
            kwargs["executable_path"] = self._memory_browser
            kwargs["firefox_user_prefs"]["framerize.captureResponseBodies"] = False
            # v3 ignores this preference; v4 skips unused debugger upload copies.
            kwargs["firefox_user_prefs"]["framerize.captureRequestBodies"] = False
            # v6 distinguishes completed requests from actual redirect chains.
            kwargs["firefox_user_prefs"]["framerize.reportActualRedirects"] = True
            # v8 also releases native caches during the existing idle GC.
            # This preference defaults off and does not unload documents.
            kwargs["firefox_user_prefs"]["framerize.reclaimGraphicsOnGC"] = (
                os.environ.get("FRAMERIZE_IDLE_CACHE_RECLAIM") == "1")
        # humanize=False (2026-10-04, measured): with True, Camoufox simulates
        # human input TIMING — every mouse.move burst cost ~19-21s inside the
        # mint lock (burst_ms=20,991 of a 21,074ms "mint"). Playwright input is
        # TRUSTED either way (Juggler Input domain), so Castle's listeners
        # still fire; we only drop the deliberate slowness.
        # Per-account egress: Castle binds tokens to egress IP (verified live).
        px = self.proxies.get(account)
        if self._observed is None:
            px = px or os.environ.get("FRAMERIZE_PROXY")
        if px:
            kwargs["proxy"] = {"server": px}
        trace(f"{account[:8]}: launching camoufox (profile {profile})")
        t0 = time.monotonic()
        # Isolated browser fixtures peaked near 1 GiB. Reserve startup
        # headroom, not the former 256 MiB estimate below even warm usage.
        # This admission estimate is not a cap on later browser growth.
        async with self._allocation(BROWSER_STARTUP_RESERVE):
            scope = self._observed_scope(account)
            life = self.lifecycles[account]
            if scope is not None:
                # Only the reserved comparison launch opts into page-owned SDK state.
                if getattr(scope, "comparison_phase", None) == "context_pending":
                    kwargs["main_world_eval"] = True
                    if self._memory_addon is None:
                        from camoufox.addons import DefaultAddons
                        kwargs["exclude_addons"] = [DefaultAddons.UBO]
                scope.begin_launch()
            try:
                if self._playwright is None:
                    await self._rpc(account, self._start_driver, async_playwright)
            except BaseException:
                if scope is not None:
                    life.phase = "unconfirmed"
                    scope.fail("driver_start_unconfirmed")
                raise
            # Each launch still has its own process, profile, configuration,
            # proxy and storage. Only the Playwright transport is shared.
            launch_state = (life.incarnation, px or None,
                            kwargs.get("main_world_eval", False) is True)
            async def launch_owned():
                created = await AsyncNewBrowser(self._playwright, **kwargs)
                # Retain both the handle and its matching cleanup receipt before
                # returning to a waiter that may already have been cancelled.
                life.browser = created
                life.close_receipt = (scope.browser_started(created) if scope is not None
                                      else BrowserCloseReceipt(created))
                return created
            try:
                browser = await self._rpc(account, launch_owned)
            except BaseException:
                # A cancelled handshake may have launched a process without
                # returning its handle. Never turn that uncertainty into room.
                life.phase = "unconfirmed"
                if scope is not None:
                    scope.fail("browser_launch_unconfirmed")
                raise
            emit("camoufox.launch", account=account[:8], duration_ms=int((time.monotonic()-t0)*1000), outcome="ok")
            try:
                acc = await self._initialize_account(account, profile, browser)
                acc.resident_launch = (browser, *launch_state)
                return acc
            except BaseException:
                if scope is not None:
                    life.phase = "unconfirmed"
                    scope.fail("account_initialization_failed")
                    raise
                acc = self.accounts.get(account)
                await asyncio.shield(self._start_close(account, life, acc))
                raise

    async def _initialize_account(self, account, profile, browser):
        scope = self._observed_scope(account)
        if scope is not None and scope.contextual_authorization:
            from contextual_authorization import initialize_account
            life = self.lifecycles[account]
            def make_account(manager, context, page):
                if (self._observed_scope(account) is not scope
                        or self.lifecycles.get(account) is not life):
                    raise RuntimeError("observed initializer ownership changed")
                return Account(manager, context, page, life)
            return await initialize_account(self, account, browser, scope, make_account)
        ctx = await self._rpc(account, browser.new_context)   # ambient context (login flow, heartbeats' home)
        page = await self._rpc(account, ctx.new_page)
        trace(f"{account[:8]}: goto login page")
        await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=90000)
        trace(f"{account[:8]}: dom loaded, waiting for app boot")
        # Gate on the APP being up (login form renders), NOT on
        # window.castlePublishableAPIKey — Firefox never gets that global.
        try:
            await self._rpc(account, page.wait_for_selector, "input[type=email], input[name=email], form", timeout=30000)
            trace(f"{account[:8]}: page ready")
        except Exception as e:
            trace(f"{account[:8]}: app-boot wait gave up ({type(e).__name__}) — continuing anyway")
        await self._rpc(account, page.wait_for_timeout, 1500)
        # Warm the Castle wrapper: the FIRST m.a() pays the v3 attempt —
        # measured 27-81s in Camoufox/Firefox (the v3 SDK's token-generation
        # can never complete in this engine; verified with module pre-warming
        # on 2026-10-02). That failure latches the wrapper's internal v3-off
        # flag, so every LATER m.a() on this page goes straight to the v2
        # fallback (~1s — Framer's own designed fallback, accepted server-side;
        # Firefox framer.com visitors get v2 too). Pay it ONCE here, hidden in
        # page setup — never inside a real mint (login + every freechat chunk).
        trace(f"{account[:8]}: warming castle wrapper (one v3-race ~6s, latches v3-off)")
        t0 = time.monotonic()
        try:
            warm_ok = await self._rpc(account, page.evaluate, WARM_JS)
            emit("castle.warm", account=account[:8], latched=bool(warm_ok),
                 duration_ms=int((time.monotonic()-t0)*1000), outcome="ok")
            trace(f"{account[:8]}: castle warm {'latched — mints now ~1s' if warm_ok else 'no token yet, latch set anyway'}")
        except Exception as e:
            trace(f"{account[:8]}: castle warm failed ({type(e).__name__}) — first real mint will pay it instead")
        acc = Account(None, ctx, page, self.lifecycles[account])
        if scope is not None:
            acc._observed_scope = scope
        acc.browser = browser
        acc.editor_file = os.path.join(profile, "framerize-editor.json")
        try:
            if os.path.exists(acc.editor_file):
                acc.editor_url = json.loads(Path(acc.editor_file).read_text()).get("editor_url")
                if acc.editor_url:
                    trace(f"{account[:8]}: editor context restored ({acc.editor_url.split('/')[-1][:12]}...)")
        except Exception:
            pass
        previous = self.accounts.get(account)
        if previous is not None:
            previous.clear_browser_http200()
            previous.resident_launch = None
        self.accounts[account] = acc
        self._track_task(acc, self._human_loop(account, page), page=page, idle_loop=True)
        # Device pool (the concurrency goose): persisted uuids per account,
        # castle-module instances warmed concurrently in the background.
        try:
            devfile = os.path.join(profile, 'framerize-devices.json')
            devices = []
            if os.path.exists(devfile):
                try:
                    devices = json.loads(Path(devfile).read_text())
                except Exception:
                    devices = []  # empty/corrupt (crash mid-write) — regenerate
            while len(devices) < DEVICE_POOL:
                devices.append(str(uuid.uuid4()))
            devices = devices[:DEVICE_POOL]  # shrink stale pools from the
            # 16-device era (they survive otherwise — Castle farm signal)
            Path(devfile).write_text(json.dumps(devices))
            acc.devices = devices
            acc.primary_dev = devices[0] if devices else None
            # Device contexts are created LAZILY on first use (each is a real
            # browser context — eager-warming N of them at boot is the slow
            # part). devpool_ready kept for compat with the token path.
            acc.devpool_ready = True
            self._track_task(acc, self._heartbeat_loop(acc, account), idle_loop=True)
        except Exception as e:
            trace(f"{account[:8]}: device pool init failed ({type(e).__name__})")
        # Device requests use their own page. Allocate the ambient mint page
        # only when the legacy mint path actually needs it.
        return acc

    async def _heartbeat_loop(self, acc, account):
        """Keep every pool device on a steady learned-as-normal cadence: one
        tiny free-route AI request per HEARTBEAT_S, cycling all devices equally
        (each device ~DEVICE_POOL x HEARTBEAT_S between beats). Waits for the
        pool to warm, skips cooling devices, uses the mint page so production
        pool pages are never blocked, and tolerates every failure silently —
        the heartbeat is pure background training, never a request source."""
        import random as _rnd
        await asyncio.sleep(45)  # let the account boot + pool warm first
        hb_idx = 0
        hb_interval = HEARTBEAT_S  # adaptive: hb 403s double it (curse decay), 200s recover
        while self.accounts.get(account) is acc and not acc.closing:
            try:
                # Operator override: ~/.framerize/heartbeat-skip.json = ["alias", ...]
                # or ["*"] — hot-reloaded each cycle (2026-10-08: an escalated
                # account's heartbeat keeps its account-level activity score
                # hot forever; skip it and let the score decay).
                try:
                    skip = json.loads(Path("~/.framerize/heartbeat-skip.json").expanduser().read_text())
                    if "*" in skip or account in skip or getattr(self, "aliases", {}).get(account) in skip:
                        await asyncio.sleep(60)
                        continue
                except Exception:
                    pass
                await asyncio.sleep(hb_interval * (0.75 + _rnd.random() * 0.5))
                async with self._activity(account):
                    if self.accounts.get(account) is not acc or acc.closing:
                        return
                    if not acc.devices or not acc.devpool_ready or not acc.project:
                        continue
                    now = time.monotonic()
                    dev = None
                    for _ in range(len(acc.devices)):
                        d = acc.devices[hb_idx % len(acc.devices)]
                        hb_idx += 1
                        if acc.dev_cooling.get(d, 0) > now:
                            continue
                        dev = d
                        break
                    if dev is None:
                        continue
                    # Fire in the DEVICE'S OWN context (2026-10-11 audit): the old
                    # path fired on the ambient mint page, whose __cuid got synced
                    # to a different device every beat — reintroducing the exact
                    # churn this architecture exists to hide, concentrated in one
                    # page's storage for the SDK to see.
                    try:
                        page = (await self._dev_page(acc, account, dev)).page
                    except Exception:
                        continue
                    tid, tok, used_dev = await self._token_acquire(acc, page, account, dev)
                    if not tok:
                        continue
                    self._note_request(acc, used_dev)
                    body = {
                        "projectId": acc.project, "featureCategory": "translation",
                        "featureName": "translation", "step": "translate",
                        "sessionId": "hb-" + uuid.uuid4().hex[:6], "stepIndex": 0,
                        "turnId": uuid.uuid4().hex, "userPrompt": "ok",
                        "request": {"model": "google/gemini-2.5-flash",
                                    "messages": [{"role": "user", "content": "ok"}],
                                    "max_tokens": 8, "stream": False},
                    }
                    job = "hb-" + uuid.uuid4().hex
                    try:
                        r = await self._run_job(page, "/ai/v3/messages/", body, tok,
                                                acc.bearer, job, used_dev, 30)
                        st = r.get("status")
                    finally:
                        self._token_release(acc, tid)
                    if st == 403:
                        self._cool_device(acc, used_dev, account)
                        # The heartbeat ITSELF got velocity-banned: this account's
                        # score is so escalated that even background beats trip it.
                        # Back off hard — every further beat just keeps the score
                        # hot. Doubling per ban (cap ~2h); recovery at half speed.
                        hb_interval = min(hb_interval * 2, 7200)
                        trace(f"{account[:8]}: heartbeat banned — interval now {int(hb_interval)}s")
                    else:
                        hb_interval = max(HEARTBEAT_S, hb_interval * 0.9)
                        if st is not None and st != 200:
                            trace(f"{account[:8]}: heartbeat got {st} (device {str(used_dev)[:8]})")
            except asyncio.CancelledError:
                raise
            except Exception:
                await asyncio.sleep(30)

    def _capture_browser_http200(self, account, acc, page, project):
        evidence = None
        try:
            acc.completed_browser_http200
            evidence = ResidentBrowserHttp200(self, account, acc, page, project)
            # Subscribe before the send yields, without making a page RPC.
            if evidence.current() and evidence.watch():
                return evidence
        except Exception:
            pass
        if evidence is not None:
            evidence.invalidate()
        return None

    def _record_browser_http200(self, acc, evidence, result):
        # The poller returns only after r.text() completes. No inference parsing.
        try:
            acc.completed_browser_http200  # Prune stale prior evidence even on refusal.
            if (evidence is not None and type(result) is dict
                    and type(result.get("status")) is int and result["status"] == 200
                    and evidence.current()):
                acc.clear_browser_http200()
                acc._completed_browser_http200 = evidence
        except Exception:
            pass
        finally:
            if evidence is not None and acc._completed_browser_http200 is not evidence:
                evidence.invalidate()

    def _ctx(self, account):
        return self.accounts[account].ctx

    def _account_for_page(self, page):
        owners = [account for account, acc in self.accounts.items()
                  if page is acc.page or page is acc.mint_page
                  or any(page is device.page for device in acc.dev_objs.values())]
        if len(owners) != 1:
            raise RuntimeError("browser page account ownership is unconfirmed")
        return owners[0]

    async def _run_job(self, page, path, body, token, bearer, job, dev, timeout):
        if self._observed is not None:
            from observed_sidecar import run_job
            scope = self._observed.scope_for_page(page)
            return await run_job(scope, page, path, body, token, bearer, job, dev,
                                 timeout, AI_START_JS, AI_POLL_JS, AI_CANCEL_JS)
        account = self._account_for_page(page)
        async with self._activity(account):
            return await self._run_job_active(account, page, path, body, token,
                                               bearer, job, dev, timeout)

    async def _run_job_active(self, account, page, path, body, token, bearer, job, dev, timeout):
        deadline = time.monotonic() + timeout
        completed = False
        try:
            await asyncio.wait_for(
                self._rpc(account, page.evaluate, AI_START_JS, [path, body, token, bearer, job, dev]),
                min(30, timeout),
            )
            emit("ai.post.started", job=job, path=path)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return {"status": -1, "usage": None, "body": f"sidecar poll timeout ({timeout:g}s)"}
                try:
                    result = await asyncio.wait_for(self._rpc(account, page.evaluate, AI_POLL_JS, job), min(8, remaining))
                except asyncio.TimeoutError:
                    result = None
                if result is not None:
                    completed = True
                    return result
                await asyncio.sleep(min(0.5, max(0, deadline - time.monotonic())))
        finally:
            if not completed:
                # abort() requests cancellation; it does not join the fetch.
                self._retain_resident_fence(self.lifecycles[account])
            # Covers success, timeout, cancellation and a broken poll call.
            # The in-page timer also covers disappearance of the host process.
            try:
                await asyncio.wait_for(self._rpc(account, page.evaluate, AI_CANCEL_JS, job), 2)
            except (Exception, asyncio.CancelledError):
                pass

    async def handle(self, req):
        if req.get("cmd") == "resident_authorization_bootstrap":
            if self._observed is not None or getattr(self, "_resident_control", None) is None:
                raise RuntimeError("resident control is unavailable")
            from resident_bootstrap import command
            return await command(self, req)
        if req.get("cmd") == "resident_authorization_once":
            control = getattr(self, "_resident_control", None)
            if self._observed is not None or control is None:
                raise RuntimeError("resident control is unavailable")
            return await control.command(req)
        account = req.get("account", "")
        # Absent status/registration are synchronous and cannot race admission.
        # Preserve their existing no-browser/no-lifecycle-allocation behavior.
        if (self._observed is not None or (account not in self.lifecycles
                and req.get("cmd") in ("register_proxy", "browser_status"))):
            return await self._handle_account_command(req)
        async with self._activity(account):
            return await self._handle_account_command(req)

    async def _handle_account_command(self, req):
        if self._observed is not None:
            req = self._observed_command(req)
        if (req.get("cmd") == "browser_baseline_once" or
                (req.get("cmd") == "prepare_inference_authorization" and "purpose" in req)):
            from contextual_authorization import (comparison_evidence, comparison_mark,
                                                   comparison_attach_progress)
            scope = None
            try:
                if self._observed is None:
                    raise RuntimeError("comparison requires observation")
                from contextual_authorization import comparison_command, validate_comparison_request
                validate_comparison_request(req)
                account = req.get("account", "")
                scope = self._observed.accounts[account]
                comparison_attach_progress(scope, req, emit)
                comparison_mark(scope, 'account_start')
                if req.get("purpose") != "baseline_context":
                    scope = self._observed.accounts[account]
                    state = scope.comparison
                    if (state is None or self.accounts.get(account) is not state["acc"]
                            or self.lifecycles[account].phase != "resident"):
                        raise RuntimeError("comparison account changed")
                async with self._use_account(account, req.get("proxy")):
                    result = await comparison_command(self, account, req)
                    emit('browser.comparison.command', outcome='returned', **comparison_evidence(scope))
                    return result
            except asyncio.CancelledError:
                raise
            except Exception:
                emit('browser.comparison.command', outcome='failed', **comparison_evidence(scope))
                raise RuntimeError("browser comparison failed") from None
        if req.get("cmd") == "prepare_inference_authorization":
            if self._observed is None:
                raise RuntimeError("contextual authorization requires an observed scope")
            from contextual_authorization import prepare, validate_request
            plan = validate_request(req)
            account = req.get("account", "")
            async with self._use_account(account, req.get("proxy")):
                return await prepare(self, account, CASTLE_CHUNK, plan)
        if req.get("cmd") == "register_proxy":
            return self._register_proxy(req)
        if req.get("cmd") == "browser_status":
            return self._browser_status(req["account"])
        if req.get("cmd") == "close_idle":
            return await self._close_idle(req)
        if req.get("cmd") in ("close", "rotate"):
            self._check_proxy_revision(req)
            account = req.get("account", "")
            if not await self._close_account(account):
                raise RuntimeError("account is busy or browser cleanup is unconfirmed")
            if req["cmd"] == "rotate":
                shutil.rmtree(os.path.join(ROOT, account), ignore_errors=True)
            return True
        if req.get("cmd") not in ("begin", "complete", "mint", "ai_post", "set_cookie", "eval"):
            raise RuntimeError("unknown cmd %s" % req.get("cmd"))
        self._adopt_proxy_route(req)
        async with self._use_account(req.get("account", ""), req.get("proxy")):
            return await self._handle(req)

    async def _handle(self, req):
        cmd = req.get("cmd")
        account = req.get("account", "")
        if cmd == "begin":
            async with self._page(account, req.get("proxy")) as page:
                trace(f"{account[:8]}: mint+signin eval start")
                t0 = time.monotonic()
                r = await self._rpc(account, page.evaluate, post_js("/auth/v4/signin", {"email": req["email"], "origin": "web"}))
                trace(f"{account[:8]}: signin -> status {r['status']}")
                ok = 200 <= r["status"] < 300
                emit("auth.signin", account=account[:8], status=r["status"],
                     duration_ms=int((time.monotonic()-t0)*1000), outcome="ok" if ok else "error",
                     **({} if ok else {"error": r["body"][:300]}))
                if not ok:
                    raise RuntimeError("signin %s: %s" % (r["status"], r["body"][:400]))
                return {"status": r["status"]}
        if cmd == "complete":
            async with self._page(account) as page:
                r = await self._rpc(account, page.evaluate, post_js("/auth/v4/continue", {"token": req["token"]}))
                if not (200 <= r["status"] < 300):
                    raise RuntimeError("continue %s: %s" % (r["status"], r["body"][:400]))
                me = await self._rpc(account, page.evaluate, """(async () => {
                  const r = await fetch("%s/web/users/me", { headers: { "x-requested-by": "Framer" }, credentials: "include" });
                  return { status: r.status, body: await r.text() };
                })()""" % API)
                if not (200 <= me["status"] < 300):
                    raise RuntimeError("me %s: %s" % (me["status"], me["body"][:400]))
                cookies = await self._rpc(account, self._ctx(account).cookies, "https://framer.com")
                session = next((c["value"] for c in cookies if c["name"] == "session"), None)
                if not session:
                    emit("auth.complete", account=account[:8], level="error", outcome="error", error="no session cookie after continue")
                    raise RuntimeError("no session cookie after continue")
                emit("auth.complete", account=account[:8], outcome="ok")
                return {"cookie_header": "session=" + session, "me": json.loads(me["body"])}
        if cmd == "mint":
            page, acc, queue_wait = await self._acquire(account)
            try:
                await self._ensure_editor(acc, page, account)
                tok, _dev = await self._mint(acc, page, account)
            finally:
                self._release(acc, page)
            if not tok:
                raise RuntimeError("castle-unavailable")
            return tok
        if cmd == "ai_post":
            acc = await self._account(account)
            page = acc.page  # ambient page (fallback only)
            queue_wait = 0.0
            t0 = time.monotonic()
            pid = (req.get("body") or {}).get("projectId")
            if pid:
                self._set_editor(acc, account, pid)
            try:
                # Mint under the per-account gate, THEN fetch unguarded: the
                # mint is the risk-engine-sensitive burst, the generation is
                # just a long I/O await that parallelizes freely.
                # VELOCITY GOOSE: pick a device by budget, fire, and if the
                # device is velocity-banned (403) cool it and INSTANTLY retry
                # on the next device — a per-device ban never reaches callers.
                attempts = 1 + min(2, len(acc.devices or [None]))
                defer_retries = req.get("defer_retries") is True
                first = req.get("attempt_index", 0) if defer_retries else 0
                if type(first) is not int or not 0 <= first < attempts:
                    raise ValueError("invalid browser attempt index")
                r = None
                for attempt in range(first, first + 1 if defer_retries else attempts):
                    dev = self._pick_device_sticky(acc) or self._pick_device(acc, account)
                    if dev is None and acc.devices:
                        # Everything cooling/over-budget: WAIT for the soonest
                        # recovery instead of poking a banned device (the poke
                        # re-bans for the full 600s — the cascade). Bounded:
                        # long waits surface as velocity-paused so the chain
                        # backs off honestly instead of minting into a ban.
                        wait = self._device_ready_in(acc)
                        if wait is not None and wait <= 60.0:
                            trace(f"{account[:8]}: all devices busy/cooling — waiting {wait:.0f}s for recovery (no re-poke)")
                            await asyncio.sleep(wait + 0.25)
                            dev = self._pick_device_sticky(acc) or self._pick_device(acc, account)
                        if dev is None:
                            w = self._device_ready_in(acc) or 0
                            r = {"status": 0, "usage": None,
                                 "body": f"velocity-paused: every device is cooling; retry in {int(w)}s"}
                            break
                    fire_page = page
                    if dev is not None and acc.browser is not None:
                        try:
                            fire_page = (await self._dev_page(acc, account, dev)).page
                        except Exception:
                            fire_page = page
                    if fire_page is page:
                        await self._ensure_editor(acc, page, account)
                    tid, tok, used_dev = await self._token_acquire(acc, fire_page, account, dev)
                    if not tok:
                        r = {"status": 0, "usage": None, "body": "castle-unavailable"}
                        break
                    try:
                        self._note_request(acc, used_dev)
                        fetch_t0 = time.monotonic()
                        job = "job-" + uuid.uuid4().hex
                        acc.project = req["body"].get("projectId") or acc.project
                        acc.bearer = req.get("bearer") or acc.bearer
                        # Deadline scales with the chunk's expected duration:
                        # a 4300-token chunk at collapse speed (~11 tok/s)
                        # legitimately needs ~390s — the flat 310s raced it and
                        # three chains died mid-generation (2026-10-05 audit).
                        # Read max_output_tokens too: the nativechain Responses
                        # payload never sets max_tokens, so the old default-600
                        # math was a lie for Codex.
                        est_s = 60.0
                        try:
                            rb = req["body"].get("request", {}) or {}
                            tok_n = rb.get("max_tokens") or rb.get("max_output_tokens") or 600
                            est_s = max(60.0, float(tok_n) / 10.0 * 3.0)
                        except Exception:
                            pass
                        evidence = self._capture_browser_http200(
                            account, acc, fire_page, req["body"].get("projectId"))
                        completed = None
                        try:
                            completed = await self._run_job(
                                fire_page, req["path"], req["body"], tok,
                                req.get("bearer"), job, used_dev, min(180.0, est_s),
                            )
                        finally:
                            self._record_browser_http200(acc, evidence, completed)
                        r = completed
                        r["_fetch_ms"] = int((time.monotonic() - fetch_t0) * 1000)
                    finally:
                        # release only after the response fully arrived — the
                        # token's request is then complete, so a later use is
                        # the proven-safe SEQUENTIAL case (never concurrent).
                        self._token_release(acc, tid)
                    # Velocity 403s are FAST (~50-150ms measured); the
                    # idle-window kill is also a 403 but arrives after 8+s of
                    # silence — retrying that on a fresh device would just die
                    # again and wrongly cool the device. Only rotate on fast 403s.
                    if r["status"] == 403 and r.get("_fetch_ms", 9999) < 4000 and attempt + 1 < attempts:
                        # TTL vs velocity (2026-10-05): TTL enforcement (~600s) makes
                        # OLD tokens 403 — that is EXPIRY, not a velocity ban, and
                        # cooling the device over it drains the pool for nothing
                        # (the cascade that killed chains today). Only a fresh
                        # token's 403 means velocity.
                        tok_age = time.monotonic() - acc.token_pool.get(tid, {}).get("born", acc.token_pool.get(tid, {}).get("ts", time.monotonic())) if tid else 0
                        if tid and tok_age > self.TOKEN_TTL_S * 0.8:
                            acc.token_pool.pop(tid, None)  # expired — refresh, same device
                            trace(f"{account[:8]}: token 403 at age {tok_age:.0f}s — TTL expiry, refreshing (device NOT cooled)")
                            if defer_retries:
                                r["_retry_device"] = True
                                break
                            continue
                        # velocity ban is per-device (proven): cool this one,
                        # rotate, retry — invisible to the caller.
                        self._cool_device(acc, used_dev, account)
                        if defer_retries:
                            r["_retry_device"] = True
                            break
                        continue
                    break
            finally:
                pass  # device-context pages are long-lived; nothing to release
            emit("ai.post", account=account[:8], path=req["path"], status=r["status"],
                 duration_ms=int((time.monotonic()-t0)*1000),
                 queue_wait_ms=int(queue_wait*1000),
                 outcome="ok" if 200 <= r["status"] < 300 else "error")
            # queue_wait_ms lets the caller separate "we waited for a page" from
            # "the upstream is slow" — they are identical in wall-clock and the
            # chunk sizer must not conflate them.
            r["queue_wait_ms"] = int(queue_wait * 1000)
            return r
        if cmd == "set_cookie":
            # Context-level op — do NOT acquire a pool page: at cap that's a
            # self-deadlock (every ai_post flow starts with set_cookie, so
            # set_cookie waiting for a free page starves behind the very
            # requests it unblocks; measured: 300s timeouts across the board).
            acc = await self._account(account)
            await self._rpc(account, self._ctx(account).add_cookies, [{
                "name": req["name"], "value": req["value"],
                "domain": ".framer.com", "path": "/",
                "secure": True, "httpOnly": True, "sameSite": "None",
            }])
            # Fan into every device context (present and future): the AI fetch
            # in a device context must carry the account session.
            if req["name"] == "session":
                acc.session_cookie = (req["name"], req["value"])
                for d in list(acc.dev_objs.values()):
                    if d.ctx is not None:
                        try:
                            await self._rpc(account, d.ctx.add_cookies, [{
                                "name": req["name"], "value": req["value"],
                                "domain": ".framer.com", "path": "/",
                                "secure": True, "httpOnly": True, "sameSite": "None",
                            }])
                        except Exception:
                            pass
            return True
        if cmd == "eval":
            if self._observed is None:
                # Arbitrary JS can schedule work after evaluation returns.
                # No generic completion proof exists for that work.
                self._retain_resident_fence(self.lifecycles[account])
            async with self._page(account) as page:
                return await self._rpc(account, page.evaluate, req["expr"])
        raise RuntimeError("unknown cmd %s" % cmd)

    async def _serve(self, req, writer):
        """Run one command and write its reply. Replies are id-matched, so they
        may be emitted out of order — the Rust side routes by id."""
        request_id = req.get("id")
        try:
            result = await self.handle(req)
            out = {"id": request_id, "ok": True, "result": result}
        except Exception as e:
            out = {"id": request_id, "ok": False, "error": str(e)[:800]}
        finally:
            # The handler owns input only until it finishes. A slow reply
            # consumer must not keep a completed request body alive.
            req = None
        # The writer owns framing and backpressure. A slow reader must not
        # block browser jobs, cancellations or the memory monitor.
        await writer.send(out)

    async def loop(self):
        if self._observed is not None:
            return await self._observed.loop(AsyncReplyWriter(sys.stdout))
        trace("sidecar ready (async)")
        loop = asyncio.get_running_loop()
        writer = AsyncReplyWriter(sys.stdout)
        # Include pending commands and detached frame writers in the idle gate.
        self._reply_writer = writer
        tasks = self._command_tasks = {}
        monitor = asyncio.create_task(self._monitor())
        try:
            while True:
                line = await loop.run_in_executor(None, sys.stdin.readline)
                try:
                    if not line:
                        break
                    if not line.strip():
                        continue
                    try:
                        try:
                            req = json.loads(line)
                        finally:
                            line = None
                        if not isinstance(req, dict):
                            raise ValueError("command must be an object")
                    except Exception as e:
                        await writer.send({"id": None, "ok": False, "error": str(e)[:800]})
                        continue
                    if req.get("cmd") == "cancel":
                        task = tasks.get(req.get("request_id"))
                        if task:
                            task.cancel()
                        continue
                    request_id = req.get("id")
                    if request_id in tasks or len(tasks) >= 512:
                        await writer.send({"id": request_id, "ok": False,
                                           "error": "sidecar busy or duplicate request id"})
                        continue
                    task = asyncio.create_task(self._serve(req, writer))
                    tasks[request_id] = task
                    task.add_done_callback(lambda done, key=request_id: tasks.pop(key, None))
                finally:
                    # The next read can wait indefinitely. Only active tasks
                    # should retain decoded input, never the reader itself.
                    line = req = task = None
        finally:
            # EOF means the owner has gone away. Waiting out its abandoned
            # requests would keep browsers and response buffers alive.
            monitor.cancel()
            pending = tuple(tasks.values())
            for task in pending:
                task.cancel()
            await asyncio.gather(monitor, *pending, return_exceptions=True)
            await writer.close()
            self._reply_writer = None
            await self.close()


# Keep this transport block identical to linear_playwright_transport.py.
import asyncio
import hashlib
import inspect
from pathlib import Path

SOURCE_SHA256 = "edac83a94c76c0f7cda78b58beaed30432fd43f611bb65480e52e160da54425b"
RUN_SHA256 = "366cfa477e221e17116cc9d2da7eb24620e7bcbd5f59a081c91fcaab67126d4d"


async def run_linear(self):
    assert self._proc.stdout
    assert self._proc.stdin
    try:
        while not self._stopped:
            try:
                header = await self._proc.stdout.readexactly(4)
                if self._stopped:
                    break
                remaining = int.from_bytes(header, byteorder="little", signed=False)
                parts = []
                while remaining:
                    size = min(remaining, 32768)
                    part = await self._proc.stdout.readexactly(size)
                    if self._stopped:
                        break
                    remaining -= size
                    parts.append(part)
                    del part
                if self._stopped:
                    break
                wire = parts[0] if len(parts) == 1 else b"".join(parts)
                parts.clear()
                del parts, header
                message = self.deserialize_message(wire)
                del wire
                self.on_message(message)
                del message
            except asyncio.IncompleteReadError:
                if not self._stopped:
                    self.on_error_future.set_exception(
                        Exception("Connection closed while reading from the driver"))
                break
            await asyncio.sleep(0)
        await self._proc.communicate()
    finally:
        if not self._stopped_future.done():
            self._stopped_future.set_result(None)


def install():
    try:
        from playwright._impl import _transport
        current = _transport.PipeTransport.run
    except (ImportError, AttributeError):
        return "unsupported"
    if current is run_linear:
        return "already_applied"
    try:
        source = Path(_transport.__file__).read_bytes()
        method = inspect.getsource(current).encode()
    except (OSError, TypeError, AttributeError):
        return "unsupported"
    if (hashlib.sha256(source).hexdigest() != SOURCE_SHA256
            or hashlib.sha256(method).hexdigest() != RUN_SHA256
            or current.__module__ != _transport.__name__):
        return "unsupported"
    _transport.PipeTransport.run = run_linear
    return "applied"


if __name__ == "__main__":
    asyncio.run(Host().loop())
