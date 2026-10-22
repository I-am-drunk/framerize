Current auth/session candidate: source-only, uncompiled, untested and undeployed.
The current integration contract is in ../../../../../SOURCE_HANDOFF.md.
It adds owned generation authorization, per-user/account version fencing,
account-first login mutation and cache invalidation. Its 38 new cases remain
unrun. The free upstream contract is complete JSON from the existing Responses
or Messages route with stream=false; upstream SSE is not an acceptance gate.
The historical package notes below describe the inherited native library.

Optional native transport library, isolated package r4 source candidate.

This copy has not been compiled or executed. The experimental-native feature
exposes framer_api::experimental_native and remains disabled by default.
It enables tokio/rt; the crate already enables Tokio time. Use a Tokio runtime
with working I/O and timers. Enabling it does not select application dispatch.

Existing native_http, sse_decoder, consumer, terminal_response,
response_adapter and native_owner APIs are unchanged. RequestPool admits at
most ten drivers and must be shared across requests. Independently constructed
pools have separate budgets. Keep RequestOwner alive for the response lifetime.
Completion confirms the original native acknowledgement and actual driver
join, independently of HTTP or AI success. Task loss and failed acknowledgement
quarantine capacity. No claim of a process memory limit follows from admission.

Raw and rejected responses retain original status, metadata and body items.
Callers must poll bodies to EOF, drop them or cancel. One absolute deadline
covers send and unpolled bodies. There is no background raw-body reader.
Request-only values are released after headers. Lower-level callers still own
send/body polling, cancellation and native cleanup observation.

downstream_owner::start uses Mode::Raw and the caller's shared pool.
from_owner attaches an existing owner without changing its mode or deadline.
Both require an explicit content-type fallback used only for an absent value.
Retain Pending.cleanup before moving Pending.response. BridgedResponse keeps
usage_header, original_content_type and rejection beside the existing proxy.
Body bytes and errors pass through unchanged. The bridge retains the owner
until real cleanup/join before final EOF. Dropping its future or body cancels.
Errors preserve their original values; cleanup remains a separate result.
Missing receivers are returned in AttachError. No task or chunk queue is added.

Retain one native_account::NativeAccountClient and clone it for callers.
Its pool is shared across accounts; separately constructed clients have
separate budgets. open_account binds AccountId, Session, limits and egress to
one generation. SuppliedAuth::new tags legitimate caller-supplied credentials
for that scope. Tagging is not proof of provenance. start takes RequestContext,
one absolute deadline and a content-type fallback. Context has no invented
identity, category or feature defaults.

close_account closes admission and cancels requests. Its CloseObserver awaits
real request cleanup/join and release of the account's client and session.
Stale scopes/auth cannot start a later generation. Before saved-session
replacement or removal, begin_session_change closes all accounts with that
active user. Retain its non-Clone ticket, await ready, perform and fully finish
the store mutation, then call finish_session_change. Concurrent same-user
mutation owners are rejected. Dropping an unfinished ticket keeps admission
closed, allowing a later owner to reacquire it. Failed cleanup stays closed.

The client bounds accounts, request records and session barriers at ten each,
with one cleanup supervisor. Preparing requests stay visible during close.
Last-client drop cancels work and reports unresolved cleanup without claiming
joins completed. Supervisor loss also closes admission and reports failure.
No library entry mints tokens, refreshes JWTs, retries refusals, persists
credentials or selects an account. Use the existing private auth boundary.

All existing cfg(test) hooks and test files are retained. New account test-only
constructors remain test-only; no fixture-root exports were added. The bridge
and account fixtures are external source candidates, not embedded test results.

provenance.json records input hashes and exact path mappings. The original
package-r3 source and evidence remain preserved at the paths it names.
The 177 previous cases and package-r3 metadata checks are historical evidence,
not validation of these additions. Fifteen bridge and eighteen account cases
and this candidate's normal/cfg(test) metadata checks remain unrun.
Application dispatch, auth-cache invalidation and saved-session integration
are separate work. Upstream acceptance, streaming/latency parity, external
unlinkability and total production memory targets remain unproven.
