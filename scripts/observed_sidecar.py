"""Explicit one-shot account teardown. Not the unmanaged or warm-serving path."""
import asyncio
import fcntl
import hashlib
import json
import os
import stat
import sys
import uuid
from pathlib import Path

MODE = "observed_one_shot_v1"
PROTOCOL = 1
MAX_RECORDS = 512
SUPPORTED = frozenset(("begin", "complete", "mint", "ai_post", "set_cookie",
                       "prepare_inference_authorization"))


def identifier(value):
    if type(value) is not str or str(uuid.UUID(value)) != value:
        raise ValueError("invalid observed identifier")
    return value


def number(value):
    if type(value) is not int or not 0 <= value < 2**64:
        raise ValueError("invalid observed sequence")
    return value


def verify_close_protocol():
    # Browser.close() in this revision suppresses TargetClosedError. Use the
    # pinned channel acknowledgement instead, and require its disconnect event.
    from playwright._impl import _browser, _connection
    pins = ((_browser, "5d087246d6a98943623a8d0bc848b27afb16cf6fb1e9ed11fb95ffd0296f2014"),
            (_connection, "1e323ddddbc6fe2c5739760059b6cd5d7cc7b1456547ec8ce44023e4774a9d6e"))
    for module, digest in pins:
        if hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() != digest:
            raise RuntimeError("unsupported observed browser close protocol")


class OwnedBrowser:
    def __init__(self, scope, browser):
        self.browser = browser
        self.scope = scope
        self.closing = self.closed = False
        self.disconnected = asyncio.Event()
        browser.on("disconnected", self._disconnected)
        if not browser.is_connected():
            self._disconnected()

    def _disconnected(self, *_):
        if not self.closing:
            self.scope.fail("browser_lost")
        self.disconnected.set()

    async def close(self):
        if self.closed:
            return
        if self.disconnected.is_set():
            raise RuntimeError("browser closed without an owned acknowledgement")
        impl = self.browser._impl_obj
        if impl._should_close_connection_on_close:
            raise RuntimeError("shared connection close is unsupported")
        self.closing = True
        await impl._channel.send("close", None, {"reason": "observed account teardown"})
        await self.disconnected.wait()
        self.closed = True
        self.browser.remove_listener("disconnected", self._disconnected)


class OwnedTask:
    """Register this record before launch; its gate excludes eager execution."""
    def __init__(self, coro, finished):
        loop = asyncio.get_running_loop()
        self.ready, self.joined = loop.create_future(), loop.create_future()
        self.coro, self.finished = coro, finished
        self.task = None
        self.started = self.cancel_requested = self.retired = False

    async def run(self):
        await self.ready
        self.started = True
        return await self.coro

    def launch(self):
        runner = self.run()
        try:
            self.task = asyncio.create_task(runner)
        except BaseException:
            # The application coroutine is behind the unopened gate, even
            # with an eager task factory. Nothing from it has run.
            runner.close()
            self.retire(None)
            raise
        self.task.add_done_callback(self.retire)
        if self.cancel_requested:
            self.cancel_once()
        if not self.ready.done():
            self.ready.set_result(None)
        return self.task

    def cancel_once(self):
        self.cancel_requested = True
        if self.task is not None and not self.task.done() and not self.task.cancelling():
            self.task.cancel()

    def retire(self, task):
        if self.retired:
            return
        self.retired = True
        if not self.started:
            self.coro.close()
        self.coro = None
        try:
            self.finished(self, task)
        finally:
            if not self.joined.done():
                self.joined.set_result(None)


class Scope:
    def __init__(self, owner, account, generation, scope_id):
        self.owner, self.account = owner, account
        self.generation, self.scope_id = generation, scope_id
        self.commands, self.background, self.rpcs = {}, set(), set()
        self.browsers = []
        self.launch_started = self.rpc_admission_closed = False
        self.contextual_authorization = False
        self.lifecycle = self.lifecycle_incarnation = None
        self.route_bound = False
        self.proxy, self.proxy_revision = None, 0
        self.sealed, self.stopped, self.changed = asyncio.Event(), asyncio.Event(), asyncio.Event()
        self.finish_ready = asyncio.Event()
        self.finish = self.seal_ack = self.supervisor = None
        self.failure = self.terminal = None
        self.terminal_sent = asyncio.Event()

    def identity(self):
        return {"account": self.account, "application_generation": self.generation,
                "scope": self.scope_id}

    def fail(self, reason):
        self.failure = self.failure or reason
        self.sealed.set()
        self.stopped.set()
        self.stop_background()

    def stop_background(self):
        for record in self.background:
            record.cancel_once()

    def seal(self, finish, stop=False):
        if self.finish is not None:
            if self.finish != finish:
                raise ValueError("scope already sealed with another token")
            if stop:
                self.stopped.set()
            self.stop_background()
            return
        self.finish = finish
        self.finish_ready.set()
        self.sealed.set()
        if stop:
            self.stopped.set()
        self.stop_background()

    def background_task(self, coro):
        if self.sealed.is_set():
            coro.close()
            done = asyncio.get_running_loop().create_future()
            done.set_result(None)
            return done
        if self.owner.background_count >= MAX_RECORDS:
            coro.close()
            raise RuntimeError("observed background capacity")
        def finished(record, done):
            self.background.discard(record)
            self.owner.background_count -= 1
            if done is not None and not done.cancelled():
                done.exception()
            self.changed.set()
        record = OwnedTask(coro, finished)
        self.background.add(record)
        self.owner.background_count += 1
        return record.launch()

    def browser_started(self, browser):
        if self.browsers:
            self.fail("multiple_account_browsers")
        self.browsers.append(OwnedBrowser(self, browser))
        return self.browsers[-1]

    def begin_launch(self):
        if self.launch_started or self.sealed.is_set():
            raise RuntimeError("observed launch admission closed")
        self.launch_started = True

    def rpc(self, coro):
        if self.rpc_admission_closed:
            coro.close()
            raise RuntimeError("observed RPC admission closed")
        if self.owner.rpc_count >= MAX_RECORDS:
            coro.close()
            raise RuntimeError("observed RPC capacity")
        def finished(record, done):
            self.rpcs.discard(record)
            self.owner.rpc_count -= 1
            if done is not None:
                if done.cancelled():
                    self.fail("rpc_cancelled")
                else:
                    done.exception()
            self.changed.set()
        record = OwnedTask(coro, finished)
        self.rpcs.add(record)
        self.owner.rpc_count += 1
        return record.launch()

    async def wait_commands(self):
        while self.commands:
            self.changed.clear()
            if self.commands:
                await self.changed.wait()

    async def supervise(self):
        try:
            await self.sealed.wait()
            await self.wait_commands()
            self.stop_background()
            await asyncio.gather(*(r.joined for r in tuple(self.background)))
            # In-flight commands may issue their final cancellation RPC while
            # stopping. Once commands/background owners join, no producer may
            # add an RPC behind the cleanup snapshot.
            self.rpc_admission_closed = True
            for browser in self.browsers:
                await browser.close()
            # A timed-out evaluate remains owned until it finishes or the
            # exact browser closes. Closing a replacement is never evidence.
            await asyncio.gather(*(r.joined for r in tuple(self.rpcs)))
            self.owner.host._observed_scope_closed(self)
            self.browsers.clear()
        except BaseException:
            self.fail("cleanup_unconfirmed")
        await self.finish_ready.wait()
        if self.seal_ack is not None:
            try:
                await asyncio.shield(self.seal_ack)
            except BaseException:
                self.fail("writer_unconfirmed")
        outcome = "unconfirmed" if self.failure else "closed"
        out = dict(self.identity(), kind="scope_cleanup", protocol=PROTOCOL,
                   incarnation=self.owner.incarnation, finish=self.finish,
                   extent="account_browser_closed", outcome=outcome)
        self.terminal = outcome
        try:
            await self.owner.writer.send(out)
            self.terminal_sent.set()
        except BaseException:
            self.fail("writer_unconfirmed")


class ObservedHost:
    def __init__(self, host, root, incarnation, profile_nonce):
        self.host = host
        self.incarnation, self.profile_nonce = identifier(incarnation), identifier(profile_nonce)
        root = Path(root)
        if root.resolve() != root.absolute() or set(p.name for p in root.iterdir()) != {".observer-owner"}:
            raise RuntimeError("observed mode needs a fresh exclusive profile root")
        self.profile_fd = os.open(root / ".observer-owner", os.O_RDWR | os.O_NOFOLLOW)
        try:
            info = os.fstat(self.profile_fd)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
                raise RuntimeError("invalid observed profile owner")
            fcntl.flock(self.profile_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if os.read(self.profile_fd, 64).decode() != self.profile_nonce:
                raise RuntimeError("observed profile owner mismatch")
        except BaseException:
            os.close(self.profile_fd)
            raise
        self.writer = None
        self.scopes, self.accounts, self.commands = {}, {}, {}
        self.controls = set()
        self.rpc_count = self.background_count = self.last_id = 0
        self.scope_limit = self.command_limit = 0
        self.negotiated = self.closed = False

    def fail(self, reason):
        self.closed = True
        for scope in self.scopes.values():
            scope.fail(reason)

    def control(self, coro):
        if len(self.controls) >= max(1, self.scope_limit + 1):
            coro.close()
            self.fail("control_capacity")
            return None
        task = asyncio.create_task(coro)
        self.controls.add(task)
        def finished(done):
            self.controls.discard(done)
            if done.cancelled() or done.exception() is not None:
                self.fail("control_writer_lost")
        task.add_done_callback(finished)
        return task

    async def reply(self, request_id, result):
        await self.writer.send({"id": request_id, "ok": True, "result": result})

    def scope_for(self, req):
        scope = self.scopes.get(identifier(req.get("scope")))
        if (scope is None or scope.account != identifier(req.get("account"))
                or scope.generation != number(req.get("application_generation"))):
            raise ValueError("observed scope identity mismatch")
        return scope

    def scope_for_page(self, page):
        browser = page.context.browser
        for scope in self.scopes.values():
            for owned in scope.browsers:
                if owned.browser is browser:
                    return scope
        raise RuntimeError("browser page has no observed owner")

    def accept(self, req):
        try:
            request_id = number(req.get("id"))
            if request_id <= self.last_id or req.get("incarnation") != self.incarnation:
                raise ValueError("stale observed command")
            self.last_id = request_id
            self._accept(req, request_id)
        except Exception:
            self.fail("protocol_error")
            request_id = req.get("id")
            if type(request_id) is not int or not 0 < request_id < 2**64:
                request_id = None
            out = {"id": request_id, "ok": False, "error": "observed protocol error"}
            self.control(self.writer.send(out))

    def _accept(self, req, request_id):
        cmd = req.get("cmd")
        if self.closed and cmd not in ("scope_stop", "scope_seal", "scope_forget"):
            raise ValueError("observed process is unavailable")
        if cmd == "observer_hello":
            if self.negotiated or self.host.accounts or self.host._playwright is not None:
                raise ValueError("unowned work precedes observation")
            scopes, commands = number(req.get("scopes")), number(req.get("commands"))
            if not scopes or not commands or scopes + commands >= MAX_RECORDS:
                raise ValueError("invalid observed capacity")
            verify_close_protocol()
            self.scope_limit, self.command_limit = scopes, commands
            self.negotiated = True
            self.control(self.reply(request_id, {"protocol": PROTOCOL, "mode": MODE,
                "incarnation": self.incarnation, "profile_nonce": self.profile_nonce,
                "initially_empty": True}))
            return
        if not self.negotiated:
            raise ValueError("observation was not negotiated")
        if cmd == "scope_open":
            account, scope_id = identifier(req.get("account")), identifier(req.get("scope"))
            generation = number(req.get("application_generation"))
            if (scope_id in self.scopes or account in self.accounts or account in self.host.accounts
                    or len(self.scopes) >= self.scope_limit):
                raise ValueError("observed account busy or scope capacity")
            scope = Scope(self, account, generation, scope_id)
            self.host._observed_scope_open(scope)
            self.scopes[scope_id] = self.accounts[account] = scope
            scope.supervisor = asyncio.create_task(scope.supervise())
            self.control(self.reply(request_id, dict(scope.identity(), opened=True)))
            return
        scope = self.scope_for(req)
        if cmd in ("scope_seal", "scope_stop"):
            finish = identifier(req.get("finish"))
            scope.seal(finish, stop=cmd == "scope_stop")
            scope.seal_ack = self.control(self.reply(request_id,
                dict(scope.identity(), sealed=True, finish=finish)))
            if scope.seal_ack is None:
                scope.fail("seal_ack_unconfirmed")
            return
        if cmd == "scope_forget":
            if req.get("finish") != scope.finish or scope.terminal != "closed" or scope.failure:
                raise ValueError("scope is not clean")
            self.control(self.forget(request_id, scope))
            return
        if cmd not in SUPPORTED or scope.sealed.is_set() or len(self.commands) >= self.command_limit:
            raise ValueError("unsupported command or closed admission")
        if cmd == "prepare_inference_authorization":
            if scope.commands or scope.launch_started or scope.contextual_authorization:
                raise ValueError("contextual authorization requires an unused scope")
            scope.contextual_authorization = True
        elif scope.contextual_authorization:
            raise ValueError("contextual authorization scope is one shot")
        gate = asyncio.Event()
        # Install reservations before the first possible poll of the task.
        self.commands[request_id] = scope.commands[request_id] = None
        task = asyncio.create_task(self.serve(req, scope, gate))
        self.commands[request_id] = scope.commands[request_id] = task
        def finished(done):
            self.commands.pop(request_id, None)
            scope.commands.pop(request_id, None)
            if done.cancelled() or done.exception() is not None:
                scope.fail("command_worker_lost")
            scope.changed.set()
        task.add_done_callback(finished)
        gate.set()

    async def forget(self, request_id, scope):
        await scope.terminal_sent.wait()
        await asyncio.shield(scope.supervisor)
        if scope.failure or scope.commands or scope.rpcs or scope.background:
            raise RuntimeError("scope still owns resources")
        self.scopes.pop(scope.scope_id)
        self.accounts.pop(scope.account)
        await self.reply(request_id, dict(scope.identity(), forgotten=True, finish=scope.finish))

    async def serve(self, req, scope, gate):
        await gate.wait()
        request_id = req["id"]
        result = None
        try:
            if scope.stopped.is_set():
                raise RuntimeError("observed scope stopped before command start")
            result = await self.host.handle(req)
            out = {"id": request_id, "ok": True, "result": result}
        except asyncio.CancelledError:
            scope.fail("command_cancelled_without_join")
            out = {"id": request_id, "ok": False, "error": "observed worker lost"}
        except Exception as exc:
            out = {"id": request_id, "ok": False, "error": str(exc)[:800]}
        finally:
            req = None
        # Stops never cancel this supervisor or its complete-frame writer.
        await self.writer.send(out)
        out = result = None

    async def loop(self, writer, stream=None):
        self.writer = writer
        self.host._reply_writer = writer
        self.host._command_tasks = self.commands
        stream = stream or sys.stdin
        loop = asyncio.get_running_loop()
        try:
            while True:
                line = await loop.run_in_executor(None, stream.readline)
                req = None
                try:
                    if not line:
                        break
                    if not line.strip():
                        continue
                    req = json.loads(line)
                    if type(req) is not dict:
                        raise ValueError("expected command object")
                    self.accept(req)
                except (ValueError, TypeError):
                    self.fail("protocol_error")
                finally:
                    line = req = None
        finally:
            await self.shutdown()

    async def shutdown(self):
        self.fail("owner_eof")
        scopes = tuple(self.scopes.values())
        for scope in scopes:
            if scope.finish is None:
                scope.seal(str(uuid.uuid4()), stop=True)
        await asyncio.gather(*(s.supervisor for s in scopes), return_exceptions=True)
        await asyncio.gather(*tuple(self.controls), return_exceptions=True)
        await self.writer.close()
        if self.host._playwright is not None:
            await self.host._playwright.stop()
        self.host._reply_writer = None
        os.close(self.profile_fd)
        self.profile_fd = None


async def run_job(scope, page, path, body, token, bearer, job, dev, timeout,
                  start_js, poll_js, cancel_js):
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    poll = None
    try:
        if scope.stopped.is_set():
            raise RuntimeError("observed scope stopped")
        start = scope.rpc(page.evaluate(start_js, [path, body, token, bearer, job, dev]))
        await asyncio.wait_for(asyncio.shield(start), min(30, timeout))
        while True:
            remaining = deadline - loop.time()
            if remaining <= 0:
                return {"status": -1, "usage": None, "body": f"sidecar poll timeout ({timeout:g}s)"}
            if scope.stopped.is_set():
                raise RuntimeError("observed scope stopped")
            if poll is None:
                poll = scope.rpc(page.evaluate(poll_js, job))
            try:
                result = await asyncio.wait_for(asyncio.shield(poll), min(8, remaining))
            except asyncio.TimeoutError:
                continue  # Reawait this exact RPC; do not lose its result.
            poll = None
            if result is not None:
                return result
            await asyncio.sleep(min(0.5, max(0, deadline - loop.time())))
    finally:
        try:
            cancel = scope.rpc(page.evaluate(cancel_js, job))
            await asyncio.wait_for(asyncio.shield(cancel), 2)
        except (Exception, asyncio.CancelledError):
            pass  # RPC ownership persists; only full browser closure is clean.
