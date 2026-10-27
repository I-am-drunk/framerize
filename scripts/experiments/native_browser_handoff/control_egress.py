"""Paired point observations, not a guarantee of future or other-destination egress."""
import asyncio
from dataclasses import dataclass
import ipaddress
import math
from pathlib import Path
import time
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
import uuid

from control_mint import ObservationRefused
from control_owner import _Binding, refuse


def _probe_url(endpoint):
    try:
        if (type(endpoint) is not str or len(endpoint) > 2048 or not endpoint.isascii()
                or any(ord(c) <= 32 or ord(c) == 127 for c in endpoint) or "\\" in endpoint):
            refuse()
        parts = urlsplit(endpoint)
        if (parts.scheme != "https" or not parts.hostname or parts.username is not None
                or parts.password is not None or parts.fragment):
            refuse()
        host = parts.hostname
        if ":" in host:
            host = "[" + str(ipaddress.IPv6Address(host)) + "]"
        if parts.port is not None and parts.port != 443:
            host += ":" + str(parts.port)
        if any(key == "framerize_probe" for key, _ in parse_qsl(parts.query)):
            refuse()
        nonce = urlencode({"framerize_probe": uuid.uuid4().hex})
        query = parts.query + "&" + nonce if parts.query else nonce
        return urlunsplit(("https", host, parts.path or "/", query, ""))
    except Exception:
        raise ObservationRefused() from None


def _address(result, url):
    try:
        if (type(result) is not dict or set(result) != {"version", "url", "ip"}
                or type(result["version"]) is not int or result["version"] != 1
                or result["url"] != url or type(result["ip"]) is not str
                or len(result["ip"]) > 45 or "%" in result["ip"]):
            refuse()
        return str(ipaddress.ip_address(result["ip"]))
    except Exception:
        raise ObservationRefused() from None


@dataclass(frozen=True, repr=False)
class NativeRouteProbe:
    """Caller-owned native route; observe returns only after its request joins."""
    account: str
    user: str
    proxy: object
    observe: object
    current: object


@dataclass(frozen=True, repr=False)
class EgressWitness:
    ip: str
    expires: float
    binding: object
    native_current: object
    stopped: object

    def __call__(self):
        try:
            return (not self.stopped.is_set() and time.monotonic() < self.expires
                and self.binding.current() is True and self.native_current() is True)
        except Exception:
            return False


@dataclass(frozen=True, repr=False)
class EgressAttester:
    endpoint: str
    native: NativeRouteProbe
    valid_seconds: float
    timeout_seconds: float = 30

    async def __call__(self, host, identity, expected, proxy, stopped, deadline):
        try:
            now = time.monotonic()
            for number in (self.valid_seconds, self.timeout_seconds, deadline):
                if type(number) not in (int, float) or not math.isfinite(number):
                    refuse()
            if (not 0 < self.valid_seconds <= 180 or not 0 < self.timeout_seconds <= 30
                    or deadline <= now or stopped.is_set()):
                refuse()
            url = _probe_url(self.endpoint)
            binding = self._binding(host, identity, expected, proxy)
        except Exception:
            raise ObservationRefused() from None
        end = min(deadline, now + self.timeout_seconds)
        return await self._owned(binding, stopped, url, end, now + self.valid_seconds)

    async def _owned(self, binding, stopped, url, deadline, expires):
        host = binding.host
        host._control_busy = True
        task = host._control_task = asyncio.create_task(
            self._observe(binding, stopped, url, deadline, expires))
        cancelled = False
        try:
            while not task.done():
                try:
                    await asyncio.shield(task)
                except asyncio.CancelledError:
                    self._stop(binding, stopped)
                    if not cancelled:
                        task.cancel()
                    cancelled = True
                except Exception:
                    break
            if cancelled:
                if not task.cancelled():
                    task.exception()
                raise asyncio.CancelledError()
            return task.result()
        finally:
            if host._control_task is task:
                host._control_busy = False
                host._control_task = None

    def _binding(self, host, identity, expected, proxy):
        native = self.native
        if (type(native) is not NativeRouteProbe or native.account != identity.account
                or native.user != expected.user or native.proxy != proxy
                or not callable(native.observe) or not callable(native.current)
                or native.current() is not True or host._control_busy
                or host._control_used or host._control_fenced):
            refuse()
        binding = _Binding(host, identity, expected)
        if binding.proxy != proxy:
            refuse()
        return binding

    @staticmethod
    def _stop(binding, stopped):
        stopped.set()
        binding.host._control_fenced = True
        binding.host._control_cancelled.set()

    async def _observe(self, binding, stopped, url, deadline, expires):
        jobs, values = [], []
        failed = cancelled = False
        host = binding.host
        host._control_busy = True
        try:
            milliseconds = min(30000, int((deadline - time.monotonic()) * 1000))
            if milliseconds < 1 or stopped.is_set() or not binding.current():
                refuse()
            script = Path(__file__).with_suffix(".js").read_text()
            request = dict(url=url, timeout_ms=milliseconds)
            jobs.append(asyncio.create_task(host._control_rpc(binding, deadline,
                binding.page.evaluate, script, request)))
            jobs.append(asyncio.create_task(self.native.observe(url, milliseconds, stopped)))
            group = asyncio.gather(*jobs, return_exceptions=True)
            values = await asyncio.wait_for(asyncio.shield(group),
                max(0, deadline - time.monotonic()))
            if any(isinstance(value, BaseException) for value in values):
                refuse()
            address = _address(values[0], url)
            if address != _address(values[1], url):
                refuse()
        except asyncio.CancelledError:
            failed = cancelled = True
        except Exception:
            failed = True
        finally:
            if failed:
                self._stop(binding, stopped)
            cleanup = asyncio.create_task(self._join(binding, jobs))
            while not cleanup.done():
                try:
                    await asyncio.shield(cleanup)
                except asyncio.CancelledError:
                    failed = cancelled = True
                    self._stop(binding, stopped)
                except Exception:
                    failed = True
                    break
            if cleanup.cancelled() or cleanup.exception() is not None or cleanup.result():
                failed = True
        if cancelled:
            raise asyncio.CancelledError()
        if failed or time.monotonic() >= deadline:
            self._stop(binding, stopped)
            refuse()
        witness = EgressWitness(address, expires,
            binding, self.native.current, stopped)
        if not witness():
            self._stop(binding, stopped)
            refuse()
        return witness

    @staticmethod
    async def _join(binding, jobs):
        values = await asyncio.gather(*(asyncio.shield(job) for job in jobs),
            return_exceptions=True)
        pending = list(binding.activity.pending)
        values += await asyncio.gather(*(asyncio.shield(job) for job in pending),
            return_exceptions=True)
        return any(isinstance(value, BaseException) for value in values)
