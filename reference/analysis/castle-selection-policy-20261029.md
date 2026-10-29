# Account selection and Castle documentation

Public documentation reviewed on 2026-10-29:

- [Browser SDK](https://docs.castle.io/docs/sdk-browser#creating-request-tokens)
  describes fresh, single-use request tokens with a 120-second lifetime.
- [Metrics](https://docs.castle.io/docs/metrics) describes configurable
  measurements across users, devices, IP addresses and event history.
- [Policies](https://docs.castle.io/docs/creating-a-policy) describes decisions
  based on those measurements. An account change does not necessarily resolve
  a security refusal involving shared IP or other signals.

These documents do not establish Framer's actual thresholds. The local
six-browser cap and rate-ledger values are product settings, not guarantees
published by Castle. A refusal category is not proof of its upstream cause.

The selector correction reuses existing availability and cooldown rules. It
balances held request leases and retains warm-browser preference on equal load.
It does not reset history, change device identity, expand security retries or
bypass the global pause. A full active pool cannot open a seventh browser.

The host's existing token reuse and 480-second lifetime differ from the public
Browser SDK contract. They require a separate check against the exact deployed
SDK and Framer wrapper. This discrepancy has not been established as the cause
of the reported refusals and is not changed by the selection repair.
