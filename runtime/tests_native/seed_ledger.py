"""Create synthetic device history for cross-language process tests."""
from pathlib import Path
import sys
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates

if __name__ == '__main__':
    with RateJournal(sys.argv[1]) as journal:
        for number in (1, 2):
            rates = AccountRates(journal, str(uuid.UUID(int=number)))
            rates.ensure_devices(2)
            rates.confirm_runtime_stopped()
