"""Conservative reservations derived from an exclusively held host snapshot."""
from copy import deepcopy
from cooldown_support import ledger

WINDOW_SECONDS = 600
ATTEMPTS = 2


def reserve(snapshot, account, devices, *, boot_id, now):
    """Require expired account history before using a separate browser context.

    The caller must already exclude the original owner. This function neither
    establishes exclusion nor authorizes a request. The original stays intact.
    """
    rows = ledger.validate(snapshot, boot_id, now, devices)
    ledger.require(account in rows and devices[account], 'comparison account unavailable')
    row = rows[account]
    ledger.require(row['primary_device'] in devices[account], 'comparison device unavailable')
    cooling = list(row['cooling'].values()) + [row['ambient_cooling'] or 0]
    queues = list(row['requests'].values()) + [row['ambient_requests'] or []]
    ledger.require(all(until <= now for until in cooling), 'comparison account cooling')
    ledger.require(row['last_mint'] + WINDOW_SECONDS <= now
                   and all(stamp + WINDOW_SECONDS <= now for q in queues for stamp in q),
                   'comparison account window has not expired')
    result = deepcopy(snapshot)
    return extend(result, account, devices, boot_id=boot_id, now=now, charge=True)


def extend(snapshot, account, devices, *, boot_id, now, charge=False):
    """Extend the hold through cleanup without erasing earlier request history.

    A separate unresolved-owner fence must remain if cleanup is unconfirmed.
    Expiration of these timestamps alone must never release that fence.
    """
    ledger.validate(snapshot, boot_id, now, devices)
    result = deepcopy(snapshot)
    row = result['state']['accounts'][account]
    for device in devices[account]:
        row['cooling'][device] = max(row['cooling'].get(device, 0), now + WINDOW_SECONDS)
        if charge:
            queue = row['requests'].setdefault(device, [])
            ledger.require(len(queue) + ATTEMPTS <= 2048, 'comparison history full')
            queue.extend([now] * ATTEMPTS)
    if row['ambient_cooling'] is not None:
        row['ambient_cooling'] = max(row['ambient_cooling'], now + WINDOW_SECONDS)
    row['last_mint'] = max(row['last_mint'], now)
    result['captured_monotonic'] = now
    ledger.validate(result, boot_id, now, devices)
    return result
