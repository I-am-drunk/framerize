"""Experimental host adapter, selected explicitly by native development tools."""
import os
from framerize_host.app import Host
from framerize_host.accounts.state import AccountLifecycle
from framerize_host.__main__ import main
from framerize_host.settings import ROOT, MINT_GAP_S, DEV_WINDOW_S, DEV_WINDOW_CAP, DEV_COOLDOWN_S
from framerize_native.device import install

if __name__ == '__main__':
    install(Host, ROOT, gap=MINT_GAP_S, window=DEV_WINDOW_S,
            capacity=DEV_WINDOW_CAP, cooldown=DEV_COOLDOWN_S,
            lifecycle_type=AccountLifecycle if os.environ.get('FRAMERIZE_NATIVE_ENROLLMENT') == '1' else None)
    main(__file__)
