"""Normal browser host entry point. Imports do not start browsers."""
import asyncio
from pathlib import Path
from framerize_runtime.release import verify_runtime

def main(script):
    verify_runtime(Path(script).resolve().parent)
    from .app import Host
    from framerize_accounts.journal import RateJournal
    from framerize_accounts.diagnostics import report, describe
    from .events import emit, trace
    from .settings import ROOT
    try:
        host = Host()
        journal = RateJournal(ROOT, recovery_policy=host.rate_policy)
    except Exception as error:
        report(error)
        emit('history.startup.failed', level='error', **describe(error))
        raise SystemExit(1) from None
    with journal:
        host.rate_journal = journal
        if journal.recovered_accounts:
            emit('history.recovered', accounts=journal.recovered_accounts,
                 cooldown_seconds=600, reserve_hold_seconds=86400)
            trace('rate history recovered; requests held for 600s, reserves held for 24h')
        asyncio.run(host.loop())

if __name__ == '__main__':
    main(__file__)
