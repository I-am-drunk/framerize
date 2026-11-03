# Accepted response fixture

`accepted-responses.ts` captures 28 responses from the pre-migration server
at commit `c26a03f6dd67b34765817897e51762dcb51ad893`.
Its `framerize-server/src/server.js` SHA256 was
`7e11aa8eac2228fedcbf028891bcd0b8907f04ecb942719008d09c07ec149242`.

The matrix covers five paths, two methods and authenticated or anonymous
requests. Eight further cases cover four backend errors on inference and
readiness. Synthetic providers supply every response. Dates and connection
headers are excluded because the HTTP runtime supplies them.

The fixture was captured before replacing the server listener with core.
Tests must not regenerate expected responses from the shared implementation:
that would compare the implementation with itself rather than preserve the
accepted HTTP behavior. Intentional protocol changes require fixture review.
