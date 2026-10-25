"""Normal browser host entry point. Imports do not start browsers."""
import asyncio
from pathlib import Path
from .release import verify_runtime

def main(script):
    package_hash = verify_runtime(Path(script).resolve().parent)
    from .app import Host
    from .accounts.state import AccountLifecycle
    from .settings import ROOT
    from .restoration import startup_state
    host = Host()
    state = Path(ROOT).resolve()
    # Legacy transfer files only belong to this exact state layout.
    if state.parent.name == '.sidecar' and state.parent.parent.name == 'profiles':
        startup_state.restore_host(host, AccountLifecycle,
            startup_state.runtime_root(state), Path(script).resolve(), package_hash)
    asyncio.run(host.loop())

if __name__ == '__main__':
    main(__file__)
