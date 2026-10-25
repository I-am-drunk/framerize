# R8 stream check and completed rollback

R8 deployed the unchanged qualified candidate and restored all eleven
accounts. It issued one acceptance request. The browser request returned
HTTP 200, but the interruption check refused before deliberately disconnecting
it because the browser request had already completed. The second acceptance
request did not run.

The retained client snapshot contains 2,175 delta-text characters, zero
terminal-text characters, zero terminal completions, no EOF and no read error.
The downstream stream was still open. This rules out terminal-response text
being mistaken for generated delta text in this attempt. It does not prove
that the upstream request was still running when that text was observed.

The selected host buffers response text before returning its command reply.
The remaining bounded source question is how that reply reaches the selected
Rust route's downstream callback. Intake owns that trace on exact 93dad965;
no additional canary is justified before resolving the ordering.

Rollback completed with all eleven account lifecycle records verified. The
previous binary and host are running, automatic restart is restored, all
owned rollout restrictions are removed, and the shared execution lock is free.
All 117 input hashes match; known controller, authentication and ingress
helpers and the transient unit are absent.

Collection:
`4faee21ad40ead658f7492a73c7a341df26fefaf603f8f36f6e48d8a3e8d6348`.
The candidate remains unaccepted. R8 is consumed and must not be replayed.
