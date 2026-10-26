"""Own existing durable device history without starting a browser."""
import asyncio
from dataclasses import dataclass, field
from pathlib import Path
import re
import uuid

from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy, SystemClock


class OwnershipError(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise OwnershipError(message)


def identifier(value):
    require(type(value) is str, 'invalid owner identity')
    try:
        parsed = uuid.UUID(value)
    except ValueError:
        raise OwnershipError('invalid owner identity') from None
    require(parsed.int != 0 and str(parsed) == value, 'invalid owner identity')
    return value


@dataclass(frozen=True, repr=False)
class Grant:
    account: str
    lease: str
    device: str
    generation: str
    def wire(self):
        return dict(generation=self.generation, lease=self.lease,
                    device=self.device, authority='ledger')


@dataclass(repr=False)
class _Lease:
    grant: Grant
    sequence: int = 0
    minting: bool = False
    waiting: bool = False


@dataclass(repr=False)
class _Account:
    rates: AccountRates
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    grants: dict = field(default_factory=dict)
    uncertain: bool = False


class LedgerOwner:
    """Ledger leases only. The caller owns session exclusion and worker joins.

    All methods run on one event loop. A grant is an in-process capability;
    wire metadata alone cannot recreate it. No browser or credential is opened.
    """
    def __init__(self, root, policy=None, clock=None, sleep=asyncio.sleep):
        history = Path(root) / 'rate-history.sqlite3'
        require(history.is_file() and not history.is_symlink(), 'existing rate history required')
        self._policy = policy or RatePolicy.from_environment()
        self._clock = clock or SystemClock()
        self._sleep = sleep
        self._generation = uuid.uuid4().hex
        self._accounts = {}
        self._closed = False
        self._clean_shutdown = False
        self._journal = RateJournal(root)

    def _live(self, grant):
        require(not self._closed, 'device owner is closed')
        require(type(grant) is Grant and grant.generation == self._generation,
                'foreign device grant')
        entry = self._accounts.get(grant.account)
        lease = entry.grants.get(grant.lease) if entry is not None else None
        require(lease is not None and lease.grant is grant,
                'foreign device grant')
        require(not entry.uncertain, 'device cleanup is unconfirmed')
        entry.rates.check_admission()
        return entry, lease

    def open(self, account, lease):
        require(not self._closed, 'device owner is closed')
        account, lease = identifier(account), identifier(lease)
        entry = self._accounts.get(account)
        if entry is None:
            require(len(self._accounts) < 64, 'device owner capacity reached')
            require(self._journal.load(account) is not None, 'existing account history required')
            entry = _Account(AccountRates(self._journal, account, self._policy, self._clock))
            self._accounts[account] = entry
        require(not entry.uncertain, 'device cleanup is unconfirmed')
        entry.rates.check_admission()
        require(lease not in entry.grants and len(entry.grants) < 64, 'device lease conflict')
        device = entry.rates.state['primary']
        if device is None:
            # A completed expiry keeps the former primary unknown. Choose and
            # persist a current device from the surviving inventory instead.
            device = entry.rates.select_primary()
        require(type(device) is str and re.fullmatch(r'[A-Za-z0-9._~-]{1,256}', device),
                'saved primary device required')
        require(device in entry.rates.ready_devices(), 'saved device is unavailable')
        grant = Grant(account, lease, device, self._generation)
        entry.rates.runtime_starting()
        entry.grants[lease] = _Lease(grant)
        return grant

    def _uncertain(self, entry):
        entry.uncertain = True
        entry.rates.mark_uncertain()

    async def admit_mint(self, grant, budget_ms):
        entry, lease = self._live(grant)
        require(type(budget_ms) is int and 1 <= budget_ms <= 16000, 'invalid mint budget')
        require(not lease.waiting and not lease.minting, 'mint already owned')
        deadline, acquired = self._clock.now() + budget_ms / 1000, False
        lease.waiting = True
        try:
            await asyncio.wait_for(entry.lock.acquire(), budget_ms / 1000)
            acquired = True
            delay = entry.rates.mint_ready_in()
            if delay:
                await asyncio.wait_for(self._sleep(delay), max(0, deadline - self._clock.now()))
            require(self._clock.now() < deadline, 'mint admission expired')
            self._live(grant)
            entry.rates.charge_request(grant.device)
            entry.rates.begin_mint()
            lease.sequence += 1
            lease.minting = True
            return lease.sequence
        finally:
            lease.waiting = False
            if acquired and not lease.minting:
                entry.lock.release()

    def finish_mint(self, grant, sequence, joined):
        entry, lease = self._live(grant)
        require(type(sequence) is int and sequence == lease.sequence and lease.minting,
                'mint identity mismatch')
        require(type(joined) is bool, 'worker cleanup evidence required')
        if not joined:
            self._uncertain(entry)
            return False
        entry.rates.finish_mint()
        lease.minting = False
        entry.lock.release()
        return (grant.device in entry.rates.state['devices']
                and entry.rates.state['cooling'].get(grant.device, 0) <= self._clock.now())

    def close_grant(self, grant, joined, risk_refused=False):
        entry, lease = self._live(grant)
        require(type(joined) is bool and type(risk_refused) is bool,
                'request cleanup evidence required')
        if risk_refused:
            entry.rates.cool_device(grant.device)
        if not joined:
            self._uncertain(entry)
            return False
        require(not lease.waiting and not lease.minting, 'mint remains owned')
        if len(entry.grants) == 1:
            entry.rates.confirm_runtime_stopped()
        del entry.grants[grant.lease]
        return True

    def shutdown(self):
        """Release storage ownership, reporting any unfinished request leases."""
        if self._closed:
            return self._clean_shutdown
        clean = all(not entry.grants and not entry.uncertain
                    and not entry.rates.failed and not entry.rates.needs_cleanup
                    for entry in self._accounts.values())
        self._closed = True
        self._journal.close()
        self._clean_shutdown = clean
        return clean
