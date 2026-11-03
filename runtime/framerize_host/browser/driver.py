"""Load and close browser dependencies at the account boundary."""
import asyncio, hashlib, os
from pathlib import Path
from ..events import emit
from ..settings import ROOT
from .transport import install
from .driver_memory import install as install_driver_memory

def browser_runtime():
    """Load browser dependencies only when an account needs a browser."""
    try:
        from camoufox.async_api import AsyncNewBrowser
        from playwright.async_api import async_playwright
    except ImportError as error:
        raise RuntimeError("camoufox not installed. Run: scripts/linux/install-camoufox.sh") from error
    install_driver_memory(ROOT)
    if os.environ.get("FRAMERIZE_LINEAR_PYTHON_PIPE", "1") == "1":
        outcome = install()
        emit("memory.python_pipe", outcome=outcome,
             level="warn" if outcome == "unsupported" else "info")
    return AsyncNewBrowser, async_playwright


async def prepare_browser_options(**kwargs):
    """Finish Camoufox configuration before a browser process can exist."""
    from camoufox.async_api import launch_options
    from camoufox.addons import DefaultAddons
    # Its default filter extension can exceed 1 GiB during startup. Framer's
    # browser transport does not need ad blocking. Keep explicit addon choices.
    kwargs.setdefault('exclude_addons', [DefaultAddons.UBO])
    options = await asyncio.to_thread(launch_options, **kwargs)
    if not isinstance(options, dict) or not options:
        raise RuntimeError("Camoufox returned no browser launch options")
    return options


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
