"""Browser allocation admission and optional patched runtime validation."""
import hashlib, json, os, sys
from pathlib import Path
from .events import emit, error_cause

NATIVE_POINTER_LIBRARY_SHA256 = "25bdc30e0afcff4190925f27f154557643645daabf9577618af750363c326096"


MEMORY_ADDON_SOURCE_SHA256 = "a7602c21732f828c1601b47ec484b76bcecbb3b7b1459ba7ca4f429f8a3037bd"


MEMORY_ADDON_PATCHED_SHA256 = "0ca79ae890b922506e4689cadfa704ab827ee803f8b1fa8e0691adf512f0a51d"


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
        result["host_total_bytes"] = mem["MemTotal"]
        result["host_reserve_bytes"] = mem["MemTotal"] // 10
        headroom.append(mem["MemAvailable"] - result["host_reserve_bytes"])
        result.update(admission_scope="host RAM used/unavailable",
                      admission_used_bytes=mem["MemTotal"] - mem["MemAvailable"],
                      admission_total_bytes=mem["MemTotal"])
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
                    remaining = limit * 9 // 10 - current
                    if not headroom or remaining < min(headroom):
                        result.update(admission_scope="limiting cgroup RAM used",
                                      admission_used_bytes=current, admission_total_bytes=limit)
                    headroom.append(remaining)
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


def memory_pressure_message(snapshot, reserved, requested):
    # The admission pair always comes from the same host or ancestor sample.
    # The older cgroup_current_bytes/cgroup_limit_bytes fields may not.
    scope = snapshot.get("admission_scope")
    used, total = snapshot.get("admission_used_bytes"), snapshot.get("admission_total_bytes")
    measured = (f"{scope}: {used / 1024**3:.2f} of {total / 1024**3:.2f} GiB"
                if scope is not None and used is not None and total is not None
                else "RAM usage unavailable")
    return ("Low available RAM: insufficient memory headroom for a new browser allocation. "
            f"{measured}; admission headroom {snapshot['allocation_headroom_bytes'] / 1024**2:.2f} MiB, "
            f"reserved {reserved / 1024**2:.2f} MiB, new allocation needs {requested / 1024**2:.2f} MiB. "
            "Retry shortly.")


class MemoryPressureError(RuntimeError):
    pass


BROWSER_STARTUP_RESERVE = 1024**3


def prepared_memory_browser():
    """Validate an optional private browser before opening accounts."""
    configured = os.environ.get("FRAMERIZE_MEMORY_BROWSER")
    automatic = not configured
    if automatic:
        from camoufox.multiversion import get_active_path
        from .browser.memory_build import digest, prepare
        from .settings import ROOT
        source = get_active_path()
        if source is None:
            raise RuntimeError("No installed Camoufox browser is selected; install it before serving requests")
        source = Path(source).resolve(strict=True)
        fingerprint = digest(source / 'omni.ja')
        target = Path(ROOT) / '.runtime' / ('browser-v11-' + fingerprint[:16])
        if not target.exists():
            prepare(source, target)
        configured = str(target)
    selected = _validate_memory_browser(configured) if configured else None
    if automatic and selected is None:
        raise RuntimeError("The prepared browser failed its memory-patch integrity check; no browser was started")
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
            10: "f37bec3303fc06b57469afabe7759c95d20d83545fb73d86672777797c6f98d9",
            11: "f37bec3303fc06b57469afabe7759c95d20d83545fb73d86672777797c6f98d9",
        }.get(version)
        if (expected_module is None
                or manifest.get("patched_module_sha256") != expected_module
                or manifest.get("preference") != "framerize.captureResponseBodies"):
            raise ValueError("unsupported memory browser patch")
        if version in (4, 5, 6, 7, 8, 9, 10, 11) and manifest.get("request_preference") != "framerize.captureRequestBodies":
            raise ValueError("unsupported request capture patch")
        if version in (6, 7, 8, 9, 10, 11) and manifest.get("redirect_preference") != "framerize.reportActualRedirects":
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
            if version in (10, 11):
                from .browser.memory_build import PROTOCOL, PROTOCOL_PATCHED
                if (manifest.get('protocol_module_sha256') != PROTOCOL_PATCHED
                        or hashlib.sha256(archive.read(PROTOCOL)).hexdigest() != PROTOCOL_PATCHED):
                    raise ValueError('browser request completion protocol changed')
            if version == 11:
                from .browser.memory_build import CONTEXTS, CONTEXTS_PATCHED
                if (manifest.get('context_module_sha256') != CONTEXTS_PATCHED
                        or hashlib.sha256(archive.read(CONTEXTS)).hexdigest() != CONTEXTS_PATCHED):
                    raise ValueError('browser context cleanup module changed')
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
        cause = error_cause(exc)
        emit("memory.browser_patch", level="warn", outcome="unavailable",
             error=type(exc).__name__, cause=cause)
        raise RuntimeError("Prepared browser integrity validation failed: " + cause) from exc
    emit("memory.browser_patch", outcome="verified", patch_version=version)
    return str(executable)


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
