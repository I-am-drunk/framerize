"""Native SDK admission against the host's existing account rate history."""
import asyncio
import hashlib
import json
import math
import os
import re
import stat
import time
import uuid
from collections import deque
from pathlib import Path
import sdk_seed_observation as sdk_seed


class Rejected(RuntimeError):
    """A refused command whose work did not outlive its reply."""


def require(value, reason):
    if not value:
        raise Rejected(reason)


def identifier(value):
    require(type(value) is str and str(uuid.UUID(value)) == value,
            'invalid native owner identity')
    return value


def finite(value):
    require(type(value) in (float, int) and math.isfinite(value) and value >= 0,
            'invalid native deadline')
    return value


def saved_devices(root, account):
    path = Path(root) / identifier(account) / 'framerize-devices.json'
    require(path.resolve(strict=True) == path, 'noncanonical device state')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()
                and 0 < info.st_size <= 16384, 'invalid saved device file')
        raw = os.read(fd, 16385)
        require(len(raw) == info.st_size and not os.read(fd, 1), 'device state changed')
        current = path.stat()
        require((current.st_dev, current.st_ino, current.st_size) ==
                (info.st_dev, info.st_ino, info.st_size), 'device state changed')
    finally:
        os.close(fd)
    devices = json.loads(raw)
    require(type(devices) is list and 0 < len(devices) <= 64
            and all(type(d) is str and 1 <= len(d) <= 256 for d in devices),
            'invalid saved device list')
    require(len(set(devices)) == len(devices), 'duplicate saved device')
    return set(devices), hashlib.sha256(raw).hexdigest()


def private_new(path, raw):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())


class Grant:
    def __init__(self, device, digest):
        self.device, self.digest = device, digest
        self.sequence = 0
        self.minting = False
        self.waiting = False
        self.uncertain = False
        self.sdk_observation = None


class NativeDevices:
    def __init__(self, host, root, *, gap, window, capacity, cooldown,
                 clock=time.monotonic, sleep=asyncio.sleep, lifecycle_type=None):
        self.host, self.root = host, Path(root).resolve(strict=True)
        self.gap, self.window, self.cooldown = map(finite, (gap, window, cooldown))
        require(self.gap <= 900 and self.window > 0 and self.cooldown > 0
                and type(capacity) is int and capacity > 0, 'invalid native rate policy')
        self.capacity, self.clock, self.sleep = capacity, clock, sleep
        self.lifecycle_type = lifecycle_type

    def enroll(self, request):
        require(not getattr(self.host, '_closed', False), 'native host is closed')
        require(self.lifecycle_type is not None, 'native enrollment unavailable')
        account = identifier(request['account'])
        enrollment = identifier(request['enrollment'])
        user, device = request['user'], request['device']
        require(type(user) is str and 0 < len(user) <= 128 and user.strip() == user,
                'invalid native login identity')
        require(type(device) is str and
                re.fullmatch(r'[A-Za-z0-9._~-]{1,256}', device),
                'invalid observed native device')
        require(account not in self.host.lifecycles and account not in self.host.accounts,
                'native account already has history')
        require(getattr(self.host, '_observed', None) is None,
                'native enrollment conflicts with observed ownership')
        user_digest = hashlib.sha256(user.encode()).hexdigest()
        require(len(self.host.lifecycles) < 64 and all(
            getattr(life, 'native_enrollment_user_sha256', None) != user_digest
            and getattr(life, 'restored_primary_device', None) != device
            for life in self.host.lifecycles.values()),
            'native enrollment identity already owned')
        directory = self.root / account
        require(not directory.exists() and not directory.is_symlink(),
                'native profile already exists')
        now = finite(self.clock())
        horizon = max(self.gap, self.window, self.cooldown)
        life = self.lifecycle_type()
        life.restored_primary_device = device
        life.native_enrollment_generation = self.host.generation
        life.native_enrollment_user_sha256 = user_digest
        life.cooling[device] = now + horizon
        life.requests[device] = deque([now] * self.capacity)
        life.last_mint = now
        directory.mkdir(mode=0o700)
        device_bytes = (json.dumps([device]) + '\n').encode()
        private_new(directory / 'framerize-devices.json', device_bytes)
        life.native_initial_device_sha256 = hashlib.sha256(device_bytes).hexdigest()
        receipt = dict(format=1, account=account, enrollment=enrollment,
                       generation=self.host.generation, enrolled_monotonic=now,
                       hold_seconds=horizon,
                       user_sha256=hashlib.sha256(user.encode()).hexdigest(),
                       device_sha256=hashlib.sha256(device.encode()).hexdigest())
        private_new(directory / 'native-enrollment.json',
                    (json.dumps(receipt, sort_keys=True) + '\n').encode())
        self.host.lifecycles[account] = life
        return dict(enrolled=True, enrollment=enrollment,
                    generation=self.host.generation,
                    cooldown_ms=math.ceil(horizon * 1000))

    def grant(self, request):
        require(not getattr(self.host, '_closed', False), 'native host is closed')
        require(request['generation'] == self.host.generation, 'stale native host')
        life = self.host.lifecycles.get(identifier(request['account']))
        require(life is not None, 'missing restored account')
        require(life.phase == 'absent' and request['account'] not in self.host.accounts
                and getattr(life, 'browser', None) is None
                and getattr(life, 'close_receipt', None) is None,
                'native account is not browserless')
        grant = getattr(life, 'native_grants', {}).get(identifier(request['lease']))
        require(grant is not None and not any(g.uncertain for g in life.native_grants.values()),
                'native ownership unavailable')
        return life, grant

    def open(self, request):
        require(not getattr(self.host, '_closed', False), 'native host is closed')
        account, lease = identifier(request['account']), identifier(request['lease'])
        restored = getattr(self.host, '_cooldown_restoration', {})
        life = self.host.lifecycles.get(account)
        initially_enrolled = (getattr(life, 'native_enrollment_generation', None)
                              == self.host.generation)
        require(restored.get('generation') == self.host.generation or initially_enrolled,
                'rate restoration required')
        require(life is not None and life.phase == 'absent'
                and account not in self.host.accounts
                and getattr(life, 'browser', None) is None
                and getattr(life, 'close_receipt', None) is None,
                'native account is not browserless')
        if not hasattr(life, 'native_grants'):
            life.native_grants, life.native_mint_lock = {}, asyncio.Lock()
        require(life.active_commands == len(life.native_grants)
                and not any(g.uncertain for g in life.native_grants.values()),
                'account has other or unconfirmed work')
        require(lease not in life.native_grants and len(life.native_grants) < 64,
                'native owner capacity or identity conflict')
        device = getattr(life, 'restored_primary_device', None)
        devices, digest = saved_devices(self.root, account)
        if initially_enrolled:
            require(digest == life.native_initial_device_sha256,
                    'initial native device state changed')
        require(type(device) is str and device in devices
                and re.fullmatch(r'[A-Za-z0-9._~-]{1,256}', device),
                'restored primary device unavailable')
        grant = Grant(device, digest)
        self.check_rate(life, grant)
        life.native_grants[lease] = grant
        life.active_commands += 1
        return dict(generation=self.host.generation, lease=lease, device=device)

    def check_rate(self, life, grant):
        now = finite(self.clock())
        require(life.cooling.get(grant.device, 0) <= now, 'device is cooling')
        queue = life.requests.setdefault(grant.device, deque())
        while queue and now - queue[0] >= self.window:
            queue.popleft()
        require(len(queue) < self.capacity, 'device request window is full')
        return now, queue

    async def mint(self, request):
        life, grant = self.grant(request)
        require(sdk_seed.matching_request(grant, request)
                and sdk_seed.ready(self.host, request['account'], life, grant),
                'native SDK observation unavailable')
        require(not grant.waiting and not grant.minting, 'native mint already owned')
        budget = request['budget_ms']
        require(type(budget) is int and 1 <= budget <= 16000, 'invalid native mint budget')
        deadline, acquired = self.clock() + budget / 1000, False
        grant.waiting = True
        try:
            await asyncio.wait_for(life.native_mint_lock.acquire(), budget / 1000)
            acquired = True
            gap = max(0, life.last_mint + self.gap - self.clock())
            if gap:
                await asyncio.wait_for(self.sleep(gap), max(0, deadline - self.clock()))
            require(self.clock() < deadline, 'native mint admission expired')
            require(self.grant(request) == (life, grant), 'native owner changed')
            require(sdk_seed.matching_request(grant, request)
                    and sdk_seed.ready(self.host, request['account'], life, grant),
                    'native SDK observation changed')
            devices, digest = saved_devices(self.root, request['account'])
            require(digest == grant.digest and grant.device in devices, 'saved device changed')
            now, queue = self.check_rate(life, grant)
            grant.sequence += 1
            queue.append(now)
            grant.minting = True
            return dict(generation=self.host.generation, lease=request['lease'],
                        sequence=grant.sequence, **sdk_seed.echo(grant))
        finally:
            grant.waiting = False
            if acquired and not grant.minting:
                life.native_mint_lock.release()

    def minted(self, request):
        life, grant = self.grant(request)
        require(sdk_seed.matching_request(grant, request), 'native SDK identity mismatch')
        require(type(request['sequence']) is int and request['sequence'] == grant.sequence
                and grant.minting, 'native mint identity mismatch')
        require(type(request['joined']) is bool, 'native cleanup evidence required')
        if not request['joined']:
            grant.uncertain = True
            return dict(confirmed=False, ready=False, **sdk_seed.echo(grant))
        life.last_mint = max(life.last_mint, finite(self.clock()))
        grant.minting = False
        life.native_mint_lock.release()
        try:
            devices, digest = saved_devices(self.root, request['account'])
            ready = grant.device in devices and digest == grant.digest
        except (OSError, ValueError, RuntimeError):
            ready = False
        ready = ready and life.cooling.get(grant.device, 0) <= self.clock()
        ready = ready and sdk_seed.ready(self.host, request['account'], life, grant)
        return dict(confirmed=True, ready=ready, **sdk_seed.echo(grant))

    def close(self, request):
        life, grant = self.grant(request)
        require(type(request['joined']) is bool and type(request['risk_refused']) is bool,
                'native completion evidence required')
        if request['risk_refused']:
            life.cooling[grant.device] = max(life.cooling.get(grant.device, 0),
                                            finite(self.clock()) + self.cooldown)
        if not request['joined']:
            grant.uncertain = True
            return dict(confirmed=False)
        require(not grant.waiting and not grant.minting, 'native mint remains owned')
        del life.native_grants[request['lease']]
        life.active_commands -= 1
        return dict(confirmed=True)

    async def handle(self, request):
        command = request['cmd']
        if command == 'native_device_enroll':
            return self.enroll(request)
        if command == 'native_device_open':
            return self.open(request)
        if command == 'native_device_sdk_seed':
            life, grant = self.grant(request)
            proof = sdk_seed.attach(self.host, request, life, grant)
            require(proof is not None, 'native SDK observation unavailable')
            return proof
        if command == 'native_device_mint':
            return await self.mint(request)
        if command == 'native_device_minted':
            return self.minted(request)
        if command == 'native_device_close':
            return self.close(request)
        raise RuntimeError('unknown native device command')


def install(host_type, root, **policy):
    from contextlib import asynccontextmanager
    host_type._sdk_seed_enabled = True
    host_type._sdk_seed = sdk_seed
    original_handle = host_type.handle
    original_use = host_type._use_account
    original_status = host_type._browser_status
    original_finish_close = host_type._finish_close

    async def finish_close(host, account, life, acc):
        try:
            closed = await original_finish_close(host, account, life, acc)
        except BaseException:
            sdk_seed.invalidate_observation(host, account)
            raise
        if closed and acc is not None:
            require(host.lifecycles.get(account) is life and life.phase == 'absent'
                    and account not in host.accounts and life.browser is None
                    and life.close_receipt is None, 'browser closure identity changed')
            # Preserve the actual final selection, after all browser work joins.
            life.restored_primary_device = acc.primary_dev
            sdk_seed.transfer_closed(host, account, life, acc)
        elif not closed:
            sdk_seed.invalidate_observation(host, account)
        return closed

    async def handle(host, request):
        if request.get('cmd', '').startswith('native_device_'):
            try:
                if not hasattr(host, '_native_devices'):
                    host._native_devices = NativeDevices(host, root, **policy)
                return await host._native_devices.handle(request)
            except (Rejected, ValueError, OSError, KeyError, asyncio.TimeoutError):
                return {'rejected': True}
        return await original_handle(host, request)

    @asynccontextmanager
    async def use_account(host, account, *args, **kwargs):
        life = host.lifecycles.get(account)
        require(not getattr(life, 'native_grants', {}), 'account has native work')
        async with original_use(host, account, *args, **kwargs) as value:
            yield value

    def status(host, account):
        value = original_status(host, account)
        life = host.lifecycles.get(account)
        if any(g.uncertain for g in getattr(life, 'native_grants', {}).values()):
            value['state'] = 'unconfirmed'
        return value

    host_type.handle, host_type._use_account = handle, use_account
    host_type._browser_status = status
    host_type._finish_close = finish_close
