# Public signup policy refusal, 25 October 2026

One diagnostic submission through Framer's existing Incognito login page at
01:22:08 UTC returned HTTP 403. The JSON error was `Forbidden`, with message
`Access denied by policy`. No email-confirmation or verification/MFA page followed.

The original page sent `POST /auth/v4/signin` with credentials included.
Its JSON contained email, origin, redirect and a nonempty Castle token.
A temporary observer forwarded the original fetch arguments unchanged,
restored fetch after that request, and retained only field names, presence
booleans and the sanitized error. It retained no credentials or token values.
The response exposed no Retry-After value. This does not prove that an
unexposed header or an internal restriction was absent.

The loaded public `login.RNUQIS3N.mjs` bundle has SHA256
`7de8ea20fd01a884bd2eb44d2fb9ed295cbd462b917a2347092ffc6bcc001a90`.
Its login handler requests a Castle token before submitting. It maps a
403 JSON error to `VERIFICATION_FAILURE`, displayed as “Verification failed”
and “Please try again”. Signup errors are similarly reduced to a generic
message. The UI therefore does not identify the rejecting policy.

The result establishes an upstream policy refusal, not a demonstrated bot,
email-domain, IP or browser-configuration cause. A nonempty token does not
prove server-side token acceptance. No client-side repair is established.
The observer itself also limits conclusions about an uninstrumented request.

No browser fingerprint, token, rate state or service configuration was
changed. Public Chrome signup remains separate from the same-host verified
session required for ordinary V2. No native mint or inference ran.
