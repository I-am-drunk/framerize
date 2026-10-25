"""Account state outlives each browser process."""
import asyncio, time

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


class AccountLifecycle:
    """Small same-host ledger; browser eviction must not reset device limits."""
    def __init__(self):
        from .activity import AccountActivity
        self.activity = AccountActivity()
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
