"""Evidence tied to one live browser and account generation."""

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
