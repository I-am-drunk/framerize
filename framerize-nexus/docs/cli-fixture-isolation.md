# CLI fixture isolation

The CLI tests now supply a temporary empty account file and explicit loopback
configuration to in-process checks. Child CLI processes receive only PATH and
SystemRoot from the host environment. Runtime code and CLI behavior are unchanged.

Previously, the focused `check validates without listening` test tried to read
the path in inherited `FRAMER_ACCOUNTS`. A baseline run with a synthetic missing
account path failed with ENOENT before listening. The signal-test children also
inherited account and host configuration.

On November 3, 2026, all nine CLI tests pass on Linux with Node.js 22.23.4 while
the parent has a synthetic missing account path, wildcard host, invalid port,
invalid upstream, and fixture credential. The new regression checks both the
child environment and explicit configuration against poisoned inputs. The
temporary test root is empty afterward.

Only the CLI test and this note change. No real account files, credentials,
inference endpoints, publication, or activation are used. Existing signal tests
still skip Windows; their Linux result does not establish macOS execution.
Actual CI results remain the next gate.
