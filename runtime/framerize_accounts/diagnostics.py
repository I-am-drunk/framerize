"""Public diagnostics contain stable reasons, never account records or secrets."""
import json
import sys


def describe(error):
    message = str(error)
    if (isinstance(error, BlockingIOError) or 'exclusive service' in message
            or 'successor service' in message or 'ownership changed' in message):
        return dict(reason='history-owner-unconfirmed',
                    action='Keep the previous runtime intact until its shutdown and exclusive ownership are confirmed.')
    if 'policy' in message:
        return dict(reason='rate-policy-mismatch',
                    action='Match the configured rate policy across migration and serving runtimes.')
    if 'schema' in message:
        return dict(reason='unsupported-history-schema',
                    action='Preserve the journal and restore a compatible database or migrate its schema.')
    if ('migration' in message or 'missing rate history' in message
            or 'existing rate history required' in message):
        return dict(reason='history-migration-required',
                    action='Start under the exclusive service owner to recover saved profiles safely.')
    if 'clock' in message:
        return dict(reason='history-clock-invalid',
                    action='Restore the matching history; do not edit boot IDs or erase cooldowns.')
    if isinstance(error, (OSError,)) or type(error).__module__ == 'sqlite3':
        return dict(reason='history-storage-unavailable',
                    action='Check journal access, free disk space, and exclusive ownership.')
    return dict(reason='account-history-invalid',
                action='Preserve saved profiles and inspect the compatible history backup.')


def report(error, event='history.startup.failed'):
    print(json.dumps(dict(event=event, level='error', **describe(error))),
          file=sys.stderr, flush=True)
