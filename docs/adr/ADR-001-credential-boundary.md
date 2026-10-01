# ADR-001: Credential boundary

Status: accepted (inherited from Notionize, adapted)

Framer login state is ONE httpOnly cookie (`session` on .framer.com, ~1y). It is
the account's only long-lived secret.

Rules:
- The cookie lives in exactly two places: the OS keychain (`framerize.session`,
  keyed by Framer user id) and the account's own WebKit session.
- Never a cookie/token column in SQLite, never in logs, never a CLI literal.
- The AI Bearer JWT (600 s) is derived state: minted from the cookie, cached in
  memory only, never persisted.
- Castle request tokens are single-shot and die in seconds; they are never stored.
