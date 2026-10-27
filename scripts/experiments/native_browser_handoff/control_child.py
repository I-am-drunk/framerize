"""Owned child-session orchestration; caller supplies actual egress attestation.

No command line, default attester, service selection or live authority is added.
The caller owns the input transport and an outer bound for broken cleanup.
"""
import asyncio
import math
import time
import uuid

from control_mint import ExpectedContext, ObservationRefused, _validate
from control_owner import refuse
from control_preparation import PreparedControlHost
from control_protocol import ControlProtocol, Frames, fields


class PreparedControlChild:
    def __init__(self, attest_egress, *, proxy_revision, prepare_seconds=180,
                 host_factory=PreparedControlHost):
        self.attest_egress, self.host_factory = attest_egress, host_factory
        self.proxy_revision, self.prepare_seconds = proxy_revision, prepare_seconds
        self.host = self.setup = None
        self.used = False
        self.stopped = asyncio.Event()

    def _request(self, request, deadline):
        fields(request, "version op binding")
        if request["op"] != "prepare":
            refuse()
        binding = request["binding"]
        fields(binding, "account user session device page_url wrapper_url post_path proxy")
        account = binding["account"]
        if (type(account) is not str or str(uuid.UUID(account)) != account
                or uuid.UUID(account).int == 0):
            refuse()
        proxy = binding["proxy"]
        if proxy is not None and (type(proxy) is not str or not proxy
                                  or len(proxy.encode()) > 16384):
            refuse()
        expected = ExpectedContext(*(binding[key] for key in
            ("user", "device", "session", "page_url", "wrapper_url", "post_path")))
        _validate(expected, deadline)
        return expected

    def _check_setup(self, deadline):
        if self.stopped.is_set() or time.monotonic() >= deadline:
            refuse()

    async def _prepare(self, request, expected, deadline):
        binding = request["binding"]
        identity = await self.host.prepare(binding["account"], expected,
            proxy=binding["proxy"], proxy_revision=self.proxy_revision, deadline=deadline)
        self._check_setup(deadline)
        current = await self.attest_egress(self.host, identity, expected,
            binding["proxy"], self.stopped, deadline)
        self._check_setup(deadline)
        if not callable(current) or current() is not True:
            refuse()
        return ControlProtocol(self.host, identity, expected, binding["proxy"], current)

    def _stop_setup(self):
        self.stopped.set()
        if self.host is not None:
            self.host._preparation_stopped.set()
            self.host._control_cancelled.set()

    async def run(self, reader, writer):
        if self.used:
            refuse()
        self.used = True
        io = Frames(reader, writer)
        cancelled = protocol_started = result = False
        try:
            seconds = self.prepare_seconds
            if (not callable(self.attest_egress) or not callable(self.host_factory)
                    or type(seconds) not in (int, float) or not math.isfinite(seconds)
                    or not 0 < seconds <= 180 or type(self.proxy_revision) is not int
                    or not 0 <= self.proxy_revision < 2 ** 64):
                refuse()
            deadline = time.monotonic() + seconds
            request = await asyncio.wait_for(io.read(), seconds)
            expected = self._request(request, deadline)
            host = self.host_factory()
            if not isinstance(host, PreparedControlHost):
                refuse()
            self.host = host
            self.setup = asyncio.create_task(self._prepare(request, expected, deadline))
            peer = await asyncio.wait_for(asyncio.shield(self.setup),
                max(0, deadline - time.monotonic()))
            self._check_setup(deadline)
            protocol_started = True
            result = await peer.run_frames(io, request)
        except asyncio.CancelledError:
            cancelled = True
        except Exception:
            pass
        finally:
            self._stop_setup()
            if not protocol_started:
                cleanup = asyncio.create_task(self._cleanup(io))
                while not cleanup.done():
                    try:
                        await asyncio.shield(cleanup)
                    except asyncio.CancelledError:
                        cancelled = True
                    except Exception:
                        break
                if not cleanup.cancelled():
                    cleanup.exception()
        if cancelled:
            raise asyncio.CancelledError()
        return result

    async def _cleanup(self, io):
        if self.setup is not None:
            await asyncio.gather(asyncio.shield(self.setup), return_exceptions=True)
        confirmed = True
        if self.host is not None:
            try:
                await self.host.close()
            except Exception:
                confirmed = False
        try:
            await io.write(dict(op="closed", grant=None, cleanup_confirmed=confirmed,
                reason="rejected" if confirmed else "unconfirmed"))
        except Exception:
            pass
        finally:
            try:
                io.writer.close()
                await io.writer.wait_closed()
            except Exception:
                pass
