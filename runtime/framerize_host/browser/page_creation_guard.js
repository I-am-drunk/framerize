// Used only by the privileged browser driver. No page or identity signals change.
class PageCreationGuard {
  constructor(window, budgetMs = 90000, closeBudgetMs = 10000) {
    this.window = window;
    this.closeBudgetMs = closeBudgetMs;
    this.phase = 'browser window startup';
    this.removers = new Set();
    this.failed = new Promise((_, reject) => { this.reject = reject; });
    this.failed.catch(() => {});
    this.fail = error => { this.error ??= error; this.reject(this.error); };
    this.closed = new Promise(resolve => { this.resolveClosed = resolve; });
    this.closeObserver = subject => {
      if (subject !== window) return;
      this.resolveClosed();
      this.fail(new Error('Browser window closed during ' + this.phase));
    };
    Services.obs.addObserver(this.closeObserver, 'domwindowclosed');
    this.timer = setTimeout(() => this.fail(new Error(
      'Firefox page creation timed out after ' + budgetMs / 1000 +
      ' seconds waiting for ' + this.phase)), budgetMs);
    if (window.closed) this.closeObserver(window);
  }

  wait(operation, phase) {
    this.phase = phase;
    if (this.error) return Promise.reject(this.error);
    return Promise.race([operation, this.failed]);
  }

  async observe(subscribe, phase) {
    let remove;
    const operation = new Promise(resolve => { remove = subscribe(resolve); });
    this.removers.add(remove);
    try { return await this.wait(operation, phase); }
    finally { remove(); this.removers.delete(remove); }
  }

  event(receiver, name, phase) {
    return this.observe(resolve => {
      receiver.addEventListener(name, resolve);
      return () => receiver.removeEventListener(name, resolve);
    }, phase);
  }

  async ready() {
    const window = this.window;
    if (window.delayedStartupPromise) {
      await this.wait(window.delayedStartupPromise, 'browser window startup');
    } else {
      await this.observe(resolve => {
        const observer = subject => { if (subject === window) resolve(); };
        Services.obs.addObserver(observer, 'browser-delayed-startup-finished');
        return () => Services.obs.removeObserver(observer, 'browser-delayed-startup-finished');
      }, 'browser window startup');
    }
    if (window.document.readyState !== 'complete')
      await this.event(window, 'load', 'browser window load');
  }

  async close() {
    if (this.window.closed) return;
    let timer;
    this.window.close();
    try {
      await Promise.race([this.closed, new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(
          'Firefox did not confirm closure of its unfinished page window')), this.closeBudgetMs);
      })]);
    } finally { clearTimeout(timer); }
  }

  dispose() {
    clearTimeout(this.timer);
    Services.obs.removeObserver(this.closeObserver, 'domwindowclosed');
    for (const remove of this.removers) remove();
    this.removers.clear();
  }
}
