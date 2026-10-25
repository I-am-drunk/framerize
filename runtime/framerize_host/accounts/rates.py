"""Account rate decisions, committed before external work begins."""
from collections import deque
from copy import deepcopy
from dataclasses import asdict, dataclass
from datetime import date
import math
from pathlib import Path
import time
import uuid


@dataclass(frozen=True)
class RatePolicy:
    window: float = 600.0
    cooldown: float = 600.0
    mint_gap: float = 0.25
    capacity: int = 180
    reserves: int = 8


class SystemClock:
    def __init__(self):
        self.boot_id = Path('/proc/sys/kernel/random/boot_id').read_text().strip()

    def now(self):
        return time.monotonic()

    def day(self):
        return time.strftime('%Y-%m-%d', time.gmtime())


class AccountRates:
    def __init__(self, journal, account, policy=None, clock=None):
        self.journal, self.account = journal, account
        self.policy, self.clock = policy or RatePolicy(), clock or SystemClock()
        self.failed = False
        self.needs_cleanup = False
        self.state = dict(devices=[], reserve=[], requests={}, cooling={},
            primary=None, reserve_day='', reserve_used=0, reserve_after=0.0,
            last_mint=0.0, pending_mint=False, clean=True, uncertain=False)
        self._validate_policy()
        record = journal.load(account) if journal is not None else None
        if record is not None:
            self._restore(record)
        elif journal is not None:
            self._commit(deepcopy(self.state))

    def _validate_policy(self):
        p = self.policy
        if (any(type(v) not in (int, float) or not math.isfinite(v) or v < 0
                for v in (p.window, p.cooldown, p.mint_gap))
                or p.window == 0 or type(p.capacity) is not int
                or not 1 <= p.capacity <= 2048 or type(p.reserves) is not int
                or not 0 <= p.reserves <= 64):
            raise ValueError('invalid account rate policy')

    def _ready(self):
        if self.failed or self.needs_cleanup:
            raise RuntimeError('account rate history requires recovery')

    def check_admission(self):
        self._ready()

    def browser_starting(self):
        self._ready()
        state = deepcopy(self.state)
        state['clean'] = False
        self._commit(state)

    def _commit(self, state):
        if self.failed:
            raise RuntimeError('account rate storage is unavailable')
        encoded = deepcopy(state)
        encoded['requests'] = [[device, list(times)]
                               for device, times in state['requests'].items()]
        encoded['cooling'] = list(state['cooling'].items())
        record = dict(version=1, boot_id=self.clock.boot_id,
                      saved_at=self.clock.now(), policy=asdict(self.policy), state=encoded)
        try:
            if self.journal is not None:
                self.journal.save(self.account, record)
        except Exception:
            self.failed = True
            raise
        for field, value in state.items():
            current = self.state[field]
            if isinstance(current, dict):
                current.clear()
                current.update(value)
            elif isinstance(current, list):
                current[:] = value
            else:
                self.state[field] = value

    @staticmethod
    def _number(value):
        if type(value) not in (float, int) or not math.isfinite(value) or value < 0:
            raise ValueError('invalid rate timestamp')
        return value

    def _device(self, device):
        if device is not None and device not in self.state['devices']:
            raise ValueError('unknown rate device')

    def _restore(self, record):
        fields = {'version', 'boot_id', 'saved_at', 'policy', 'state'}
        if (type(record) is not dict or set(record) != fields
                or type(record['version']) is not int or record['version'] != 1
                or type(record['boot_id']) is not str or not record['boot_id']
                or record['policy'] != asdict(self.policy)):
            raise ValueError('incompatible account rate history')
        saved = self._number(record['saved_at'])
        state = self._decode(record['state'], saved)
        now = self.clock.now()
        if record['boot_id'] == self.clock.boot_id:
            if saved > now:
                raise ValueError('account rate clock moved backwards')
            self.state = state
        else:
            self._rebase(state, saved, now)
            self._commit(state)
        self.needs_cleanup = not state['clean']

    @staticmethod
    def _pairs(rows, devices):
        if type(rows) is not list or len(rows) > 65:
            raise ValueError('invalid rate device rows')
        result = {}
        for row in rows:
            if (type(row) is not list or len(row) != 2
                    or (row[0] is not None and row[0] not in devices)
                    or row[0] in result):
                raise ValueError('invalid rate device row')
            result[row[0]] = row[1]
        return result

    def _decode(self, raw, saved):
        if type(raw) is not dict or set(raw) != set(self.state):
            raise ValueError('invalid account rate record')
        state = deepcopy(raw)
        devices = state['devices']
        if (type(devices) is not list or len(devices) > 64
                or any(type(d) is not str or not 1 <= len(d) <= 256 for d in devices)
                or len(set(devices)) != len(devices)):
            raise ValueError('invalid saved device inventory')
        reserve = state['reserve']
        if (type(reserve) is not list or any(d not in devices for d in reserve)
                or len(set(reserve)) != len(reserve)
                or state['primary'] is not None and state['primary'] not in devices):
            raise ValueError('invalid saved device selection')
        if (type(state['reserve_used']) is not int
                or not 0 <= state['reserve_used'] <= self.policy.reserves):
            raise ValueError('invalid saved reserve count')
        if state['reserve_day'] != '':
            date.fromisoformat(state['reserve_day'])
        for field in ('clean', 'uncertain', 'pending_mint'):
            if type(state[field]) is not bool:
                raise ValueError('invalid saved activity state')
        if state['clean'] and (state['uncertain'] or state['pending_mint']):
            raise ValueError('inconsistent saved activity state')
        if self._number(state['last_mint']) > saved:
            raise ValueError('future saved mint')
        self._number(state['reserve_after'])
        state['requests'] = self._pairs(state['requests'], devices)
        state['cooling'] = self._pairs(state['cooling'], devices)
        for device, stamps in state['requests'].items():
            if (type(stamps) is not list or len(stamps) > 2048
                    or any(self._number(t) > saved for t in stamps)
                    or stamps != sorted(stamps)):
                raise ValueError('invalid saved request window')
            state['requests'][device] = deque(stamps)
        for until in state['cooling'].values():
            if self._number(until) > saved + self.policy.cooldown:
                raise ValueError('invalid saved cooldown')
        return state

    def _rebase(self, state, saved, now):
        # Downtime across boots is uncertain. Preserve remaining restrictions.
        state['cooling'] = {d: now + max(0, t - saved)
                            for d, t in state['cooling'].items()}
        state['requests'] = {d: deque(max(0, now - (saved - t)) for t in stamps
                                     if saved - t < self.policy.window)
                             for d, stamps in state['requests'].items()}
        state['last_mint'] = max(0, now - (saved - state['last_mint']))
        if state['reserve_used']:
            state['reserve_after'] = now + 86400
        else:
            state['reserve_after'] = now

    def ensure_devices(self, count):
        self._ready()
        if type(count) is not int or not 0 <= count <= 64:
            raise ValueError('invalid device pool size')
        if not self.state['devices'] and count:
            state = deepcopy(self.state)
            state['devices'] = [str(uuid.uuid4()) for _ in range(count)]
            state['primary'] = state['devices'][0]
            self._commit(state)
        return tuple(self.state['devices'])

    def _recent(self, device, now):
        return deque(t for t in self.state['requests'].get(device, ())
                     if now - t < self.policy.window)

    def ready_devices(self):
        self._ready()
        now = self.clock.now()
        return [d for d in self.state['devices']
                if self.state['cooling'].get(d, 0) <= now
                and len(self._recent(d, now)) < self.policy.capacity]

    def select_primary(self):
        ready = self.ready_devices()
        chosen = self.state['primary']
        if chosen not in ready:
            chosen = ready[0] if ready else None
        if chosen != self.state['primary']:
            state = deepcopy(self.state)
            state['primary'] = chosen
            self._commit(state)
        return chosen

    def allocate_reserve(self):
        self._ready()
        state = deepcopy(self.state)
        now, today = self.clock.now(), self.clock.day()
        if today > state['reserve_day'] and now >= state['reserve_after']:
            state.update(reserve_day=today, reserve_used=0, reserve_after=now + 86400)
        if state['reserve_used'] >= self.policy.reserves or len(state['devices']) >= 64:
            return None
        device = str(uuid.uuid4())
        state['devices'].append(device)
        state['reserve'].append(device)
        state['primary'] = device
        state['reserve_used'] += 1
        self._commit(state)
        return device

    def charge_request(self, device):
        self._ready()
        self._device(device)
        now = self.clock.now()
        requests = self._recent(device, now)
        if (self.state['cooling'].get(device, 0) > now
                or len(requests) >= self.policy.capacity):
            raise RuntimeError('device request budget unavailable')
        state = deepcopy(self.state)
        requests.append(now)
        state['requests'][device] = requests
        state['clean'] = False
        self._commit(state)

    def cool_device(self, device):
        self._ready()
        self._device(device)
        state = deepcopy(self.state)
        state['cooling'][device] = max(state['cooling'].get(device, 0),
                                      self.clock.now() + self.policy.cooldown)
        state['clean'] = False
        self._commit(state)

    def mint_ready_in(self):
        self._ready()
        if self.state['pending_mint']:
            raise RuntimeError('account already owns an unfinished mint')
        return max(0, self.state['last_mint'] + self.policy.mint_gap - self.clock.now())

    def begin_mint(self):
        if self.mint_ready_in() > 0:
            raise RuntimeError('account mint pacing unavailable')
        state = deepcopy(self.state)
        state.update(last_mint=self.clock.now(), pending_mint=True, clean=False)
        self._commit(state)

    def finish_mint(self):
        self._ready()
        if not self.state['pending_mint']:
            raise RuntimeError('account has no owned mint')
        state = deepcopy(self.state)
        state.update(last_mint=self.clock.now(), pending_mint=False)
        self._commit(state)

    def mark_uncertain(self):
        state = deepcopy(self.state)
        state.update(uncertain=True, clean=False)
        self._commit(state)

    def confirm_browser_closed(self):
        """Call only after the account's actual browser cleanup is confirmed."""
        state = deepcopy(self.state)
        now = self.clock.now()
        if self.needs_cleanup or state['uncertain'] or state['pending_mint']:
            for device in [None, *state['devices']]:
                state['cooling'][device] = now + self.policy.cooldown
            state['last_mint'] = now
        state.update(pending_mint=False, uncertain=False, clean=True)
        self._commit(state)
        self.needs_cleanup = False

    def device_ready_in(self):
        self._ready()
        now, waits = self.clock.now(), []
        for device in self.state['devices']:
            wait = max(0, self.state['cooling'].get(device, 0) - now)
            requests = self._recent(device, now)
            if len(requests) >= self.policy.capacity:
                wait = max(wait, requests[0] + self.policy.window - now)
            waits.append(wait)
        return min(waits, default=0)
