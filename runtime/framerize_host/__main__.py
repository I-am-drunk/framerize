"""Normal browser host entry point. Imports do not start browsers."""
import asyncio
from pathlib import Path
from .release import verify_runtime

def main(script):
    verify_runtime(Path(script).resolve().parent)
    from .app import Host
    from .accounts.journal import RateJournal
    from .settings import ROOT
    with RateJournal(ROOT) as journal:
        asyncio.run(Host(journal).loop())

if __name__ == '__main__':
    main(__file__)
