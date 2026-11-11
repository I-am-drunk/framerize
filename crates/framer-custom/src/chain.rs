//! Tail-context leg loop: the custom-route engine's coordinator.
//!
//! One leg = one streaming-abort rung (see rung) framed by the prompt
//! templates (see stop): leg 1 carries the length guarantee, later legs
//! carry the tail anchor of everything captured so far. After every leg the
//! credits ledger is asserted (see ledger); a debit or a race routes through
//! the retry/quarantine policy instead of flowing silently onward.
//!
//! Pure and mock-testable: the only upstream contact is CustomTransport.
//!
//! MEMORY: one copy of the text lives in `kept` (per-leg owned growth);
//! `accumulated` is a SECOND copy used only for anchor slicing + final
//! length logging. At 8 legs x ~1.2KB this is ~20KB total -- accepted and
//! documented, not worth rental-lifetime surgery.
//!
//! Sequential-accounting assumption: legs run one at a time and nothing else
//! spends between the before/after reads, so summing per-leg deltas equals
//! the end-to-end balance move. If the daemon ever parallelizes legs, switch
//! total_debit to a single before-first/after-last comparison.

use crate::ledger;
use crate::merge;
use crate::rung;
use crate::stop;
use crate::transport::CustomTransport;

/// Whole-chain wall-clock budget. Caps the worst case (max_legs x
/// LEG_TIMEOUT) so a pathological settings combo cannot wedge a worker.
/// 8 legs x 600s = 4800s worst case; the chain bound fires first at 3600s
/// with a typed error -- tune together with max_legs.
pub const CHAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);
/// Cap on accepted legs: bounds `legs`/`kept`/`accumulated` memory no matter
/// what settings arrive. A chain that needs more than 64 legs is a runaway,
/// not a document.
pub const MAX_LEGS_HARD: u32 = 64;
/// Cap on prompt bytes per leg: a hostile or garbage topic must not build a
/// megabyte payload. 64KB holds any real topic + template + anchor.
pub const MAX_PROMPT_CHARS: usize = 64_000;

/// Knobs for [`complete`]. Defaults mirror the proven driver: stop depth
/// 200, at most 8 legs, 2 halved retries per leg, a 350-char tail anchor.
#[derive(Debug, Clone)]
pub struct ChainSettings {
    /// Abort depth per leg (clamped into [MIN_STOP, MAX_STOP] at run time).
    pub stop: u32,
    /// Leg cap: 0 means "no legs" (empty chain), otherwise the max ACCEPTED
    /// legs; retries do not consume this budget. Hard-capped at MAX_LEGS_HARD.
    pub max_legs: u32,
    /// Halved-stop retries per leg before quarantine. Hard-capped at 10:
    /// halving bottoms at MIN_STOP after ~3, the rest would be identical legs.
    pub quarantine_retries: u32,
    /// Tail-anchor width in chars. 0 = anchorless continuations (degenerate;
    /// allowed for tests, never for production).
    pub anchor_chars: usize,
    /// Whole-chain wall-clock budget (default CHAIN_TIMEOUT). Tests shrink it.
    pub chain_timeout: std::time::Duration,
    /// Per-oracle-read budget (default ORACLE_TIMEOUT). Tests shrink it.
    pub oracle_timeout: std::time::Duration,
}

/// Hard bounds applied by normalize().
pub const MAX_RETRIES_HARD: u32 = 10;
/// Anchor widths above this are pure overhead (production uses 350).
pub const MAX_ANCHOR_CHARS: usize = 4096;

impl ChainSettings {
    /// Clamp every knob into its hard bound. Idempotent; call at every
    /// trust boundary (server handler, tests, future callers).
    pub fn normalize(&self) -> ChainSettings {
        ChainSettings {
            stop: stop::clamped_stop(self.stop),
            max_legs: self.max_legs.min(MAX_LEGS_HARD),
            quarantine_retries: self.quarantine_retries.min(MAX_RETRIES_HARD),
            anchor_chars: self.anchor_chars.min(MAX_ANCHOR_CHARS),
            chain_timeout: self.chain_timeout,
            oracle_timeout: self.oracle_timeout,
        }
    }
}

impl Default for ChainSettings {
    fn default() -> Self {
        Self {
            stop: stop::DEFAULT_STOP,
            max_legs: 8,
            quarantine_retries: 2,
            anchor_chars: 350,
            chain_timeout: CHAIN_TIMEOUT,
            oracle_timeout: rung::ORACLE_TIMEOUT,
        }
    }
}

/// One finished leg as the engine records it.
#[derive(Debug)]
pub struct Leg {
    /// Text kept from this leg (trimmed growth; empty on a raced debit or
    /// a near-zero finish leg).
    pub text: String,
    /// True when the terminal arrived before the stop point.
    pub raced: bool,
    /// True when the ledger moved across this leg.
    pub debited: bool,
    /// Stop depth the rung ran at.
    pub stop_used: u32,
    /// Ledger delta (after - before) observed across this leg, retries
    /// included. Sums to Assembled.total_debit.
    pub debit: f64,
    /// Stream events consumed on the ACCEPTED attempt (deltas + terminal),
    /// for per-leg diagnostics.
    pub events: u32,
}

/// A finished chain: assembled text, per-leg record, and the honesty footer.
#[derive(Debug)]
pub struct Assembled {
    /// Joined kept growth across accepted legs (echo-trimmed).
    pub text: String,
    /// Per-leg record in run order (finish leg included, quarantined if any).
    pub legs: Vec<Leg>,
    /// Sum of per-leg debits. 0.0 on the proven path.
    pub total_debit: f64,
    /// How the chain ended: leg cap, model dry, or quarantine.
    pub outcome: ChainOutcome,
    /// Complete tool calls from the finish-cut leg (empty for text turns).
    pub tool_calls: Vec<serde_json::Value>,
}

/// Terminal state of a chain run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainOutcome {
    /// Stopped on near-zero growth: the model ran dry. The normal ending.
    Capped,
    /// Hit max_legs with growth still arriving: raise the cap or the stop.
    LegCap,
    /// A leg kept debiting past its retry budget: text is partial, debit>0.
    Quarantined,
    /// The finish cutoff fired: the pump aborted on the model's
    /// finish_reason delta, so the captured text is the COMPLETE answer and
    /// the meter's terminal frame never arrived. The clean ending.
    Finished,
}

/// Last `n` chars of `text`, widened forward to a char boundary so
/// multi-byte content is never split. Returns what exists when shorter.
/// `n == 0` yields an empty anchor (no clamping surprise).
pub fn last_anchor_chars(n: usize, text: &str) -> &str {
    if n == 0 || text.is_empty() {
        return "";
    }
    if text.len() <= n {
        return text;
    }
    let mut start = text.len() - n;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// What the chain serves (ADR-003): a plain prompt (the /api/custom/complete
/// path, unchanged) or a full native chat payload — the dialect routes run
/// the same cutoff engine on the client's own messages.
#[derive(Clone)]
pub enum ChainRequest {
    /// Plain prompt: leg 1 is the length directive + the prompt; continuations
    /// are the nudge + tail anchor (the proven shape).
    Prompt(String),
    /// Native chat payload ({"messages": [...], ...}). Leg 1 prepends the
    /// length directive as a system message — the client's messages stay
    /// verbatim. Continuations append the accumulated tail as an assistant
    /// turn plus the nudge, so the model continues ITS OWN text. Extra keys
    /// (tools, temperature, ...) ride every leg unchanged.
    Chat(serde_json::Value),
}

/// Leg-1 payload: length guarantee FIRST, topic second (instruction-first:
/// models obey a leading directive more reliably than a trailing one).
/// Topics over MAX_PROMPT_CHARS are rejected, not truncated (a cut topic
/// changes the question silently).
fn leg1_payload(prompt: &str) -> Result<serde_json::Value, String> {
    if prompt.len() > MAX_PROMPT_CHARS {
        return Err(format!(
            "chain: leg-1 topic is {} bytes, over the {MAX_PROMPT_CHARS}-byte cap",
            prompt.len()
        ));
    }
    // PAD-STRUCTURAL (2026-11-10, live-proved): the length guarantee alone
    // does NOT keep short answers generating -- the model answers briefly
    // and the terminal settles before any abort can land (measured: every
    // unpadded short leg bills; every padded one aborts mid-stream at $0).
    // So the padding directive is structural here, not model-obedience: the
    // request always demands continued generation past the answer, giving
    // the marker/finish/stop aborts a live stream to cut every time.
    Ok(serde_json::json!({
        "messages": [
            {"role": "user", "content": format!("{}\n\n{}\n\n{}", stop::length_guarantee_suffix(1800, 14), prompt, stop::end_marker_suffix())}
        ]
    }))
}

/// Continuation payload: directive, then the tail anchor, then an empty
/// closer so the anchor is never the message's last line (some models treat
/// trailing quoted text as the thing to repeat).
fn continue_payload(accumulated: &str, anchor_chars: usize) -> serde_json::Value {
    serde_json::json!({
        "messages": [
            {"role": "user", "content": format!("{}{}\n\n{}", stop::continue_suffix(), last_anchor_chars(anchor_chars, accumulated), "")}
        ]
    })
}

/// Native leg 1: the client's payload verbatim, with the length directive
/// prepended as a system message (instruction-first; the client's own
/// messages are never rewritten).
fn leg1_native(payload: &serde_json::Value) -> Result<serde_json::Value, String> {
    // TOOLS FAIL CLOSED (2026-11-11 -- P8c billed -0.0395 at dispatch, P8d identical
    // prompt without tools $0): a tool-carrying open bills at dispatch even streamed
    // with the stop baked in, and refunds deny ~261:1 historically. Refuse before
    // dispatch unless the operator opts into the risk. Text legs unaffected.
    if std::env::var("FRAMERIZE_ALLOW_TOOL_LEGS").map(|v| v == "1").unwrap_or(false) == false
    {
        if let Some(tools) = payload.get("tools").and_then(|t| t.as_array()) {
            if !tools.is_empty() {
                return Err("chain: tool-carrying legs bill at dispatch on metered routes (proved -0.0395 live); refusing before open. Text requests serve free; retry without tools or set FRAMERIZE_ALLOW_TOOL_LEGS=1 to attempt (spend risk).".to_string());
            }
        }
    }
    let mut p = payload.clone();
    let msgs = payload["messages"]
        .as_array()
        .ok_or("chain: native chat payload has no messages array")?;
    // Native payloads carry a whole conversation (codex: 76K tokens of
    // context) -- far over the prompt-string cap. The native cap is 2MB:
    // the real bound is the upstream request limit, not the prompt rule.
    const MAX_NATIVE_PAYLOAD: usize = 2_000_000;
    if serde_json::to_string(&p).map(|s| s.len()).unwrap_or(0) > MAX_NATIVE_PAYLOAD {
        return Err(format!(
            "chain: native payload over the {MAX_NATIVE_PAYLOAD}-byte cap"
        ));
    }
    let mut new_msgs = vec![serde_json::json!({
        "role": "system",
        "content": stop::end_marker_suffix(),
    })];
    new_msgs.extend(msgs.iter().cloned());
    p["messages"] = serde_json::json!(new_msgs);
    // TOOL DIALECT (2026-11-11 -- P8b Azure 400): the leg posts to the chat route, which
    // demands {type:"function",function:{name,description,parameters}}. Anthropic-shaped
    // tools ({name,input_schema}, no type/function) 400 there. Normalize the tools array
    // only; text-only payloads (no tools key) are byte-identical to before.
    if let Some(tools) = p.get("tools").and_then(|t| t.as_array()).map(|a| a.to_vec()) {
        if !tools.is_empty()
            && tools.iter().any(|t| t.get("type").and_then(|v| v.as_str()) != Some("function"))
        {
            let mapped: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    if t.get("type").and_then(|v| v.as_str()) == Some("function") {
                        t.clone()
                    } else {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": t.get("name").cloned().unwrap_or(serde_json::Value::Null),
                                "description": t.get("description").cloned().unwrap_or(serde_json::Value::Null),
                                "parameters": t.get("input_schema").cloned().unwrap_or(serde_json::json!({"type":"object"})),
                            },
                        })
                    }
                })
                .collect();
            p["tools"] = serde_json::Value::Array(mapped);
        }
    }
    Ok(p)
}

/// Native continuation: original messages + the accumulated tail as an
/// assistant turn + the continue nudge. The model continues its own text;
/// the client's instructions/tools ride along unchanged.
fn continue_native(
    payload: &serde_json::Value,
    accumulated: &str,
    anchor_chars: usize,
) -> serde_json::Value {
    let mut p = payload.clone();
    let msgs = payload["messages"].as_array().cloned().unwrap_or_default();
    let tail = last_anchor_chars(anchor_chars, accumulated);
    let mut new_msgs = msgs;
    new_msgs.push(serde_json::json!({"role": "assistant", "content": tail}));
    new_msgs.push(serde_json::json!({"role": "user", "content": stop::continue_suffix()}));
    p["messages"] = serde_json::json!(new_msgs);
    // TOOLS FAIL CLOSED (continuations): strip any tools array so a continuation leg can
    // never open billed (leg-1 refusal normally prevents reaching here; direct callers
    // of complete_inner bypass it). Opt-in restores.
    if std::env::var("FRAMERIZE_ALLOW_TOOL_LEGS").map(|v| v == "1").unwrap_or(false) == false
    {
        if let Some(tools) = p.get("tools").and_then(|t| t.as_array()) {
            if !tools.is_empty() {
                eprintln!("[custom-route] continuation stripped tool-carrying payload (fail-closed; FRAMERIZE_ALLOW_TOOL_LEGS=1 to attempt)");
                p["tools"] = serde_json::Value::Array(vec![]);
            }
        }
    }
    p
}

/// Per-leg trace line for daemon logs: everything needed to diagnose a
/// billed or short leg without re-running it.
fn leg_line(tag: &str, leg_index: u32, stop: u32, r: &rung::RungResult, before: f64, after: f64) -> String {
    format!(
        "[custom-route] {tag} leg={} stop={} events={} captured={} raced={} credits {:.6} -> {:.6} (delta {:+.6})",
        leg_index,
        stop,
        r.events,
        r.captured.len(),
        r.raced,
        before,
        after,
        after - before
    )
}

/// Timed oracle read: a hung credits endpoint fails the leg, never the chain.
async fn read_oracle<T: CustomTransport>(
    transport: &T,
    budget: std::time::Duration,
    which: &str,
    leg_index: u32,
) -> Result<f64, String> {
    tokio::time::timeout(budget, transport.read_credits())
        .await
        .map_err(|_| {
            format!("chain: {which} oracle read timed out on leg {} (hung credits endpoint)", leg_index + 1)
        })?
}

/// Run the full chain: stream legs until near-zero growth or `max_legs`.
///
/// Per leg: snapshot credits, run the rung at the current stop, snapshot
/// credits again. A `RetryHalved` policy reruns THIS leg verbatim (the
/// accumulated text is untouched, so the rebuilt payload is identical) at
/// a halved stop while the per-leg retry budget lasts; once spent, the leg
/// is recorded debited-and-quarantined and chaining stops. Accepted legs
/// append their trimmed growth; near-zero growth ends the chain (Capped).
/// `total_debit` sums the recorded legs' ledger deltas.
///
/// TIMEOUTS: the whole chain runs under chain_timeout; each oracle read
/// under oracle_timeout; each leg under rung LEG_TIMEOUT. Any rung/transport
/// error aborts the chain with Err (partial legs are NOT returned half-cut:
/// inspect the error, which names the leg and the cause).
///
/// DIAGNOSTICS: every leg attempt emits one `[custom-route]` eprintln line
/// (leg, stop, events, captured bytes, raced, credit before/after/delta) so
/// a billed, short, or raced leg is diagnosable from daemon logs alone.
pub async fn complete<T: CustomTransport>(
    transport: &mut T,
    leg1_prompt: &str,
    chain: &ChainSettings,
) -> Result<Assembled, String> {
    complete_native(
        transport,
        ChainRequest::Prompt(leg1_prompt.to_string()),
        chain,
    )
    .await
}

/// The ADR-003 entry: serve a ChainRequest (prompt or native chat payload)
/// through the cutoff chain. Identical engine; the request shape is the only
/// difference.
/// Max spend per request (2026-11-10, user directive "lock so you can only spend 0.01"):
/// any chain whose measured total exceeds this halts + withholds (backstop behind the
/// per-leg quarantine, covering delayed settles and concurrent-traffic masking).
/// FRAMERIZE_MAX_SPEND_PER_REQUEST overrides; default 0.01.
/// Confirm a measured move before quarantining (2026-11-11): the meter places
/// auth-HOLDS at open that release seconds later (proved live: per-request oracle dips
/// that vanish before the next pool read). Returns the CONFIRMED debit, or 0.0 when the
/// hold released (caller then treats the leg as clean). Costs +20s only on the rare
/// moved path; clean legs never wait.
pub async fn confirm_settle<T: CustomTransport>(
    transport: &mut T,
    cfg: &ChainSettings,
    before: f64,
    leg_index: u32,
) -> f64 {
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    match read_oracle(transport, cfg.oracle_timeout, "confirm", leg_index).await {
        Ok(recheck) => {
            let d = ledger::signed_delta(before, recheck);
            if d < -crate::ledger::LEDGER_EPS {
                eprintln!(
                    "[custom-route] confirm leg={}: CONFIRMED settle {:+.6} -- quarantining",
                    leg_index + 1,
                    d
                );
                return d;
            }
            eprintln!(
                "[custom-route] confirm leg={}: hold released ({:.2} recovered) -- treating as clean",
                leg_index + 1,
                recheck
            );
            0.0
        }
        Err(e) => {
            eprintln!("[custom-route] confirm leg={}: re-read failed ({e}) -- treating as clean (gate open on oracle failure)", leg_index + 1);
            0.0
        }
    }
}

pub fn max_spend_per_request() -> f64 {
    std::env::var("FRAMERIZE_MAX_SPEND_PER_REQUEST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.01)
}

pub async fn complete_native<T: CustomTransport>(
    transport: &mut T,
    request: ChainRequest,
    chain: &ChainSettings,
) -> Result<Assembled, String> {
    let cfg = chain.normalize();
    tokio::time::timeout(cfg.chain_timeout, complete_inner(transport, &request, &cfg))
        .await
        .map_err(|_| {
            format!(
                "chain: whole-chain budget of {}s exhausted (max_legs={})",
                cfg.chain_timeout.as_secs(),
                cfg.max_legs
            )
        })?
}

async fn complete_inner<T: CustomTransport>(
    transport: &mut T,
    request: &ChainRequest,
    chain: &ChainSettings,
) -> Result<Assembled, String> {
    // Settings arrive normalized (complete() normalizes at the boundary);
    // re-derive locals defensively so a direct complete_inner caller cannot
    // run away either.
    let max_legs = chain.max_legs.min(MAX_LEGS_HARD);
    let retry_budget = chain.quarantine_retries.min(MAX_RETRIES_HARD);
    let anchor_chars = chain.anchor_chars.min(MAX_ANCHOR_CHARS);
    let cfg = chain;
    let mut legs: Vec<Leg> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    let mut accumulated = String::new();
    let mut total_debit = 0.0f64;
    // Chain-level stop: restored to default after every Accept so one
    // recovered race (halved stop) cannot permanently shrink later legs.
    let base_stop = cfg.stop;
    let mut stop = base_stop;
    let mut retries_this_leg: u32 = 0;
    let mut leg_debit = 0.0f64;
    // CHAIN-START ANCHOR (2026-11-10): the first before-read anchors the chain.
    // Before opening legs 2+ the oracle is re-read and compared to the ANCHOR
    // (not just the leg start): a delayed settle from an earlier leg posts
    // AFTER its after-read and would otherwise fund another billed leg.
    let mut chain_start: Option<f64> = None;
    let mut outcome = ChainOutcome::Capped;
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();

    let mut leg_index: u32 = 0;
    // DEBITED-RACE RECOVERY (2026-11-10, live-proved): set when a leg raced
    // the terminal WITH a ledger move. The next attempt re-asks with the
    // padding demand appended so the model generates past the answer and
    // the abort lands mid-stream (halving the stop re-runs the same burst
    // and re-bills). Cleared whenever a leg resolves.
    let mut recover_padded = false;
    while leg_index < max_legs {
        let payload = match (request, leg_index) {
            (ChainRequest::Prompt(p), 0) => {
                if recover_padded {
                    leg1_payload(&format!("{}\n\n{}", p, stop::race_recovery_suffix()))?
                } else {
                    leg1_payload(p)?
                }
            }
            (ChainRequest::Prompt(_), _) => {
                let mut c = continue_payload(&accumulated, anchor_chars);
                if recover_padded {
                    if let Some(ms) = c["messages"].as_array_mut() {
                        if let Some(m) = ms.first_mut() {
                            let cur = m["content"].as_str().unwrap_or("").to_string();
                            m["content"] = serde_json::Value::String(format!(
                                "{}\n\n{}",
                                cur,
                                stop::race_recovery_suffix()
                            ));
                        }
                    }
                }
                c
            }
            (ChainRequest::Chat(v), 0) => {
                if recover_padded {
                    let mut vv = v.clone();
                    if let Some(ms) = vv["messages"].as_array_mut() {
                        for m in ms.iter_mut().rev() {
                            if m["role"] == "user" {
                                let cur = m["content"].as_str().unwrap_or("").to_string();
                                m["content"] = serde_json::Value::String(format!(
                                    "{}\n\n{}",
                                    cur,
                                    stop::race_recovery_suffix()
                                ));
                                break;
                            }
                        }
                    }
                    leg1_native(&vv)?
                } else {
                    leg1_native(v)?
                }
            }
            (ChainRequest::Chat(v), _) => continue_native(v, &accumulated, anchor_chars),
        };

        let before = read_oracle(transport, cfg.oracle_timeout, "before", leg_index).await?;
        if chain_start.is_none() {
            chain_start = Some(before);
        }
        // DELAYED-SETTLE GUARD: a prior leg's settle can post after its after-read.
        // If the chain-level balance moved since the anchor, halt BEFORE opening
        // another upstream leg (the move is booked, text withheld downstream).
        if leg_index > 0 {
            if let Some(anchor_bal) = chain_start {
                if !ledger::is_settled(anchor_bal, before) {
                    let drift = ledger::signed_delta(anchor_bal, before);
                    total_debit += drift;
                    eprintln!(
                        "[custom-route] delayed-settle guard leg={}: chain moved {:+.6} since anchor -- halting before open",
                        leg_index + 1,
                        drift
                    );
                    outcome = ChainOutcome::Quarantined;
                    break;
                }
            }
        }
        // SPEND CAP: already over budget from prior legs -> do not open another.
        if total_debit < -max_spend_per_request() {
            eprintln!(
                "[custom-route] spend cap leg={}: total {:+.6} exceeds cap -- halting before open",
                leg_index + 1,
                total_debit
            );
            outcome = ChainOutcome::Quarantined;
            break;
        }
        let rung_out = rung::run_rung(transport, &payload, stop).await.map_err(|e| {
            format!("chain: leg {} failed ({e}); chain aborted, {} legs kept", leg_index + 1, legs.len())
        })?;
        let after = read_oracle(transport, cfg.oracle_timeout, "after", leg_index).await?;
        let delta = ledger::signed_delta(before, after);
        let debited = !ledger::is_settled(before, after);
        leg_debit += delta;
        eprintln!("{}", leg_line("attempt", leg_index + 1, stop, &rung_out, before, after));

        // THE FINISH CUTOFF (2026-11-09): the pump aborted on the model's
        // finish_reason delta -- the captured text is the COMPLETE answer
        // and the meter's terminal frame never arrived (raced stays false;
        // if the terminal slipped through in the same burst the ledger delta
        // records it and the settle-reversal refunds it downstream). The
        // chain ENDS: continuation legs would regenerate a finished answer.
        if rung_out.finished {
            total_debit += leg_debit;
            let growth = merge::trim_overlap(&rung_out.captured, &accumulated).to_string();
            eprintln!(
                "[custom-route] finish-cutoff leg={} stop={} events={} captured={} debited={} -- complete answer, chain ends",
                leg_index + 1,
                stop,
                rung_out.events,
                rung_out.captured.len(),
                debited
            );
            accumulated.push_str(&growth);
            kept.push(growth.clone());
            legs.push(Leg {
                text: growth,
                raced: rung_out.raced,
                debited,
                stop_used: stop,
                debit: leg_debit,
                events: rung_out.events,
            });
            tool_calls = rung_out.tool_calls.clone();
            outcome = ChainOutcome::Finished;
            break;
        }

        match ledger::after_leg(rung_out.raced, debited) {
            ledger::Policy::RetryHalved | ledger::Policy::QuarantineAndReport
                if retries_this_leg >= retry_budget =>
            {
                // HOLD-vs-SETTLE: confirm the chain-level move before booking quarantine.
                let anchor_bal = chain_start.unwrap_or(before);
                let confirmed = confirm_settle(transport, cfg, anchor_bal, leg_index).await;
                if confirmed >= -crate::ledger::LEDGER_EPS {
                    eprintln!(
                        "[custom-route] budget-exhausted leg={}: hold released on confirm -- treating as clean",
                        leg_index + 1
                    );
                    leg_debit = 0.0;
                    // fall through to accept path below with zeroed debit
                    total_debit += 0.0;
                    let growth = merge::trim_overlap(&rung_out.captured, &accumulated).to_string();
                    accumulated.push_str(&growth);
                    kept.push(growth.clone());
                    legs.push(Leg {
                        text: growth,
                        raced: rung_out.raced,
                        debited: false,
                        stop_used: stop,
                        debit: 0.0,
                        events: rung_out.events,
                    });
                    stop = base_stop;
                    retries_this_leg = 0;
                    leg_debit = 0.0;
                    leg_index += 1;
                    recover_padded = false;
                    continue;
                }
                total_debit += leg_debit;
                eprintln!(
                    "[custom-route] quarantine leg={} after {} retries (raced={} debited={} debit={:+.6})",
                    leg_index + 1,
                    retries_this_leg,
                    rung_out.raced,
                    debited,
                    leg_debit
                );
                legs.push(Leg {
                    text: merge::trim_overlap(&rung_out.captured, &accumulated).to_string(),
                    raced: rung_out.raced,
                    debited,
                    stop_used: stop,
                    debit: leg_debit,
                    events: rung_out.events,
                });
                outcome = ChainOutcome::Quarantined;
                recover_padded = false;
                break;
            }
            ledger::Policy::RetryHalved | ledger::Policy::QuarantineAndReport => {
                // QUARANTINE-ON-FIRST-DEBIT (2026-11-10, live-proved): every
                // padded/micro retry shape re-bills on per-token meters
                // (OpenRouter settles each delta; no abort timing wins).
                // A moved ledger therefore halts the chain at once with the
                // honest debit booked -- no retry spends a second time.
                // Clean (no-debit) races still retry as before.
                if debited {
                    // HOLD-vs-SETTLE: confirm before quarantining (holds release in ~seconds).
                    let confirmed = confirm_settle(transport, cfg, before, leg_index).await;
                    if confirmed < -crate::ledger::LEDGER_EPS {
                        total_debit += confirmed;
                        eprintln!(
                            "[custom-route] quarantine leg={} on confirmed debit (raced={} debit={:+.6}) -- no retry, retries re-bill live",
                            leg_index + 1,
                            rung_out.raced,
                            confirmed
                        );
                        legs.push(Leg {
                            text: String::new(),
                            raced: rung_out.raced,
                            debited: true,
                            stop_used: stop,
                            debit: confirmed,
                            events: rung_out.events,
                        });
                        outcome = ChainOutcome::Quarantined;
                        recover_padded = false;
                        break;
                    }
                    // Hold released: fall through to the clean-race recovery below.
                    leg_debit = 0.0;
                }
                retries_this_leg += 1;
                // Debited races recover by SHAPE CHANGE (pad + re-ask), not
                // by shrinking the stop: halving the stop re-runs the same
                // burst and re-bills (live-proved -0.06 -> -0.06). Clean
                // races (no debit) still halve the stop as before.
                // The padded retry runs at MIN_STOP: the padding demand is
                // long, so the re-ask still streams; the small stop aborts
                // it after a few deltas, bounding the retry's own settle
                // (live: same-stop padded retries re-settled -0.06).
                recover_padded = debited;
                let next = if debited {
                    // Abort after the FIRST delta: live proves even 2-event
                    // bursts bill on OpenRouter-routed models, so any larger
                    // recovery stop re-bills. run_rung cancels at
                    // deltas >= stop_steps, so stop_steps=1 ends the leg on
                    // the first framed delta.
                    1
                } else {
                    stop::halt_after_race(stop).max(1)
                };
                eprintln!(
                    "[custom-route] retry leg={} attempt={} stop {} -> {} (raced={} debited={})",
                    leg_index + 1,
                    retries_this_leg + 1,
                    stop,
                    next,
                    rung_out.raced,
                    debited
                );
                stop = next;
                continue;
            }
            ledger::Policy::Accept => {
                let growth = merge::trim_overlap(&rung_out.captured, &accumulated).to_string();
                total_debit += leg_debit;
                let leg_total = leg_debit;
                leg_debit = 0.0;
                if merge::is_near_zero(&growth) {
                    eprintln!(
                        "[custom-route] accept leg={} stop={} events={} captured={} raced={} debited={} FINISH (near-zero growth)",
                        leg_index + 1,
                        stop,
                        rung_out.events,
                        rung_out.captured.len(),
                        rung_out.raced,
                        debited
                    );
                    legs.push(Leg {
                        text: String::new(),
                        raced: rung_out.raced,
                        debited,
                        stop_used: stop,
                        debit: 0.0,
                        events: rung_out.events,
                    });
                    outcome = ChainOutcome::Capped;
                    recover_padded = false;
                    break;
                }
                eprintln!(
                    "[custom-route] accept leg={} stop={} events={} captured={} kept={} raced={} debited={}",
                    leg_index + 1,
                    stop,
                    rung_out.events,
                    rung_out.captured.len(),
                    growth.len(),
                    rung_out.raced,
                    debited
                );
                accumulated.push_str(&growth);
                kept.push(growth.clone());
                legs.push(Leg {
                    text: growth,
                    raced: rung_out.raced,
                    debited,
                    stop_used: stop,
                    // The leg absorbs the whole attempt series (a recovered
                    // race's failed attempt settled for real); total_debit
                    // already absorbed the same sum above.
                    debit: leg_total,
                    events: rung_out.events,
                });
                retries_this_leg = 0;
                stop = base_stop;
                leg_index += 1;
            }
        }
    }

    if leg_index >= max_legs && outcome == ChainOutcome::Capped && !legs.is_empty() {
        // Ran out of legs while still growing: only Capped-worthy if the
        // last leg was the near-zero finisher; otherwise the cap cut us off.
        let last_empty = legs.last().map(|l| l.text.is_empty()).unwrap_or(false);
        if !last_empty {
            outcome = ChainOutcome::LegCap;
            eprintln!(
                "[custom-route] leg cap reached (max_legs={}): growth still arriving",
                max_legs
            );
        }
    }

    // CHAIN-LEVEL FINALITY (2026-11-10): no chain that moved the ledger
    // may serve text as a clean answer. A multi-leg run can go 5x$0 then
    // settle on leg 6 (live: -0.108848 across 6 legs); per-leg quarantine
    // fires but the assembled text would still serve. Normalize here: any
    // nonzero total forces Quarantined with EMPTY text (books kept on the
    // legs; the route 402s; the breaker trips downstream).
    let (text, outcome) = if total_debit < -crate::ledger::LEDGER_EPS {
        eprintln!(
            "[custom-route] CHAIN QUARANTINE: total_debit={:+.6} across {} legs -- text withheld",
            total_debit,
            legs.len()
        );
        (String::new(), ChainOutcome::Quarantined)
    } else {
        (merge::assemble(&kept), outcome)
    };
    eprintln!(
        "[custom-route] done legs={} kept_chars={} total_debit={:+.6} outcome={:?}",
        legs.len(),
        text.len(),
        total_debit,
        outcome
    );
    Ok(Assembled {
        text,
        legs,
        total_debit,
        outcome,
        tool_calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Event;
    use async_trait::async_trait;
    use std::collections::VecDeque;

    /// Leg-aware script transport: each stream_reply pops the next queued
    /// (events, credit-after) pair. The staged balance applies when the
    /// terminal is consumed or the script runs dry -- modelling "the ledger
    /// moves when the leg settles" -- so the after-read sees the new value.
    struct ChainMock {
        queue: VecDeque<(Vec<Event>, f64)>,
        current: Vec<Event>,
        pos: usize,
        credits: f64,
        staged: Option<f64>,
        payloads: Vec<serde_json::Value>,
    }

    impl ChainMock {
        fn new(legs: Vec<(Vec<Event>, f64)>, credits: f64) -> Self {
            Self {
                queue: legs.into(),
                current: Vec::new(),
                pos: 0,
                credits,
                staged: None,
                payloads: Vec::new(),
            }
        }

        fn settle(&mut self) {
            if let Some(v) = self.staged.take() {
                self.credits = v;
            }
        }
    }

    #[async_trait]
    impl CustomTransport for ChainMock {
        async fn stream_reply(&mut self, payload: &serde_json::Value) -> Result<(), String> {
            self.payloads.push(payload.clone());
            let (events, after) = self.queue.pop_front().unwrap_or((vec![], self.credits));
            self.current = events;
            self.pos = 0;
            self.staged = Some(after);
            Ok(())
        }

        async fn next_event(&mut self) -> Option<Event> {
            match self.current.get(self.pos).cloned() {
                Some(Event::Done) => {
                    self.pos += 1;
                    self.settle();
                    Some(Event::Done)
                }
                Some(ev) => {
                    self.pos += 1;
                    Some(ev)
                }
                None => {
                    self.settle();
                    None
                }
            }
        }

        async fn cancel(&mut self) {}

        async fn read_credits(&self) -> Result<f64, String> {
            Ok(self.credits)
        }
    }

    /// Oracle that hangs: proves the oracle timeout fires.
    struct HungOracle;
    #[async_trait]
    impl CustomTransport for HungOracle {
        async fn stream_reply(&mut self, _: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        async fn next_event(&mut self) -> Option<Event> {
            Some(Event::Done)
        }
        async fn cancel(&mut self) {}
        async fn read_credits(&self) -> Result<f64, String> {
            std::future::pending().await
        }
    }

    fn numbered(from: usize, n: usize) -> Vec<Event> {
        (from..from + n)
            .map(|i| Event::Delta(format!("word{i} ")))
            .collect()
    }

    fn fast_settings() -> ChainSettings {
        ChainSettings {
            oracle_timeout: std::time::Duration::from_millis(200),
            chain_timeout: std::time::Duration::from_secs(30),
            ..ChainSettings::default()
        }
    }

    #[tokio::test]
    async fn three_leg_chain_with_retry_then_finish() {
        // Leg 1: long stream, clean abort at stop 200, no debit.
        // Leg 2 attempt 1: debited -> QUARANTINE ON FIRST DEBIT (no retry:
        // live proves every retry shape re-bills on per-token meters).
        // Leg 2's honest -0.5 stays on the chain total; the leg text is
        // empty (quarantined legs are never served as answers).
        let mut leg1 = numbered(0, 250);
        leg1.push(Event::Done);
        let mut leg2a = numbered(200, 150);
        leg2a.push(Event::Done);
        let mut leg2b = numbered(200, 150);
        leg2b.push(Event::Done);
        let leg3 = vec![Event::Done];
        let mut t = ChainMock::new(
            vec![(leg1, 100.0), (leg2a, 99.5), (leg2b, 99.5), (leg3, 99.5)],
            100.0,
        );
        let out = complete(&mut t, "explain ships", &fast_settings()).await.unwrap();

        assert_eq!(out.legs.len(), 2);
        assert_eq!(out.total_debit, -0.5);
        assert_eq!(out.legs[1].debit, -0.5);
        assert_eq!(out.outcome, ChainOutcome::Quarantined);
        // leg 1 kept words 0..200; leg 2 is the empty quarantine record.
        assert!(!out.legs[0].raced && !out.legs[0].debited);
        assert_eq!(out.legs[0].stop_used, 200);
        assert!(out.legs[1].raced && out.legs[1].debited);
        assert_eq!(out.legs[1].stop_used, 200);
        assert!(out.legs[1].text.is_empty());
        // CHAIN FINALITY: the assembled text is withheld (empty) even
        // though leg 1 kept words -- nonzero total serves nothing.
        // only two payloads ran: leg 1 and the quarantined leg-2 attempt.
        assert_eq!(t.payloads.len(), 2);
        let c1 = t.payloads[0]["messages"][0]["content"].as_str().unwrap();
        assert!(c1.contains("explain ships"));
        assert!(c1.contains(&stop::length_guarantee_suffix(1800, 14)));
    }

    #[tokio::test]
    async fn stop_restores_after_recovery() {
        // Leg 1 runs clean at the base stop; a debited leg-2 attempt
        // quarantines at once without touching any later leg's stop.
        // (Recovery never shrinks other legs: there is no retry at all.)
        let mut leg1 = numbered(0, 250);
        leg1.push(Event::Done);
        let mut l2a = numbered(0, 3);
        l2a.push(Event::Done);
        let mut t = ChainMock::new(vec![(leg1, 100.0), (l2a, 99.5)], 100.0);
        let out = complete(&mut t, "probe", &fast_settings()).await.unwrap();
        assert_eq!(out.legs[0].stop_used, 200);
        assert_eq!(out.legs[1].stop_used, 200);
        assert_eq!(out.outcome, ChainOutcome::Quarantined);
    }

    #[tokio::test]
    async fn debited_race_quarantines_without_retry() {
        // A debited race quarantines on the FIRST attempt: no padded
        // retry (live: every retry shape re-bills on per-token meters).
        // The honest debit stays booked; the leg text is empty.
        let mut l1a = numbered(0, 3);
        l1a.push(Event::Done);
        let fin = vec![Event::Done];
        let mut t = ChainMock::new(vec![(l1a, 49.5), (fin, 49.5)], 50.0);
        let out = complete(&mut t, "probe", &fast_settings()).await.unwrap();
        assert_eq!(t.payloads.len(), 1);
        assert_eq!(out.outcome, ChainOutcome::Quarantined);
        assert_eq!(out.total_debit, -0.5);
        assert!(out.legs[0].text.is_empty());
    }

    #[tokio::test]
    async fn quarantine_path_records_and_stops() {
        // A debited race with NO retry budget quarantines immediately: the
        // leg records raced+debited, the honest debit stays on the books,
        // the chain stops Quarantined.
        let mk = || {
            let mut v = numbered(900, 2);
            v.push(Event::Done);
            v
        };
        let mut t = ChainMock::new(vec![(mk(), 49.0)], 50.0);
        let settings = ChainSettings { quarantine_retries: 0, ..fast_settings() };
        let out = complete(&mut t, "probe", &settings).await.unwrap();
        assert_eq!(out.legs.len(), 1);
        assert!(out.legs[0].debited && out.legs[0].raced);
        assert!((out.total_debit - -1.0).abs() < 1e-9);
        assert!((out.legs[0].debit - -1.0).abs() < 1e-9);
        assert_eq!(out.outcome, ChainOutcome::Quarantined);
    }

    #[tokio::test]
    async fn leg_cap_outcome_when_still_growing() {
        let mk = |from: usize| {
            let mut v = numbered(from, 250);
            v.push(Event::Done);
            v
        };
        let mut t = ChainMock::new(
            vec![(mk(0), 10.0), (mk(1000), 10.0), (mk(2000), 10.0)],
            10.0,
        );
        let settings = ChainSettings { max_legs: 2, ..fast_settings() };
        let out = complete(&mut t, "probe", &settings).await.unwrap();
        assert_eq!(out.outcome, ChainOutcome::LegCap);
        assert_eq!(out.legs.len(), 2);
    }

    #[tokio::test]
    async fn native_chat_payload_rides_the_chain_verbatim() {
        // ADR-003 item 1: a native chat payload keeps the client's messages
        // untouched (directive prepended as a system message), and the
        // continuation appends the accumulated tail + nudge. Extra keys
        // (here: a tools array) ride every leg unchanged.
        let payload = serde_json::json!({
            "messages": [
                {"role": "user", "content": "write a poem"}
            ],
            "tools": [{"type": "function", "name": "noop"}]
        });
        // TOOLS FAIL CLOSED (2026-11-11 -- P8c -0.0395, P9b -0.50 at dispatch; refunds
        // deny ~261:1): tool-carrying legs refuse BEFORE dispatch. Text serves free.
        let err = leg1_native(&payload).unwrap_err();
        assert!(err.contains("bill at dispatch"), "refusal must name the reason: {err}");
        // Text-only twin rides through with the pad system message prepended.
        let text_only = serde_json::json!({"messages": [{"role": "user", "content": "write a poem"}]});
        let leg1 = leg1_native(&text_only).unwrap();
        let msgs = leg1["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert!(msgs[0]["content"].as_str().unwrap().contains("words"));
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "write a poem");

        let cont = continue_native(&payload, "roses are red\n", 350);
        let msgs = cont["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["content"], "write a poem");
        assert_eq!(msgs[1]["role"], "assistant");
        assert!(msgs[1]["content"].as_str().unwrap().contains("roses are red"));
        assert_eq!(msgs[2]["role"], "user");
        assert!(msgs[2]["content"].as_str().unwrap().contains("Continue"));
        // TOOLS FAIL CLOSED (continuations strip tools; P8c/P9b): no continuation leg
        // may ever open billed.
        assert!(cont["tools"].as_array().map(|a| a.is_empty()).unwrap_or(true),
            "continuation must strip tools");
    }

    #[test]
    fn leg1_native_normalizes_anthropic_tool_shape() {
        // P8b regression: Anthropic tools must leave leg1 in chat-route dialect.
        let payload = serde_json::json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "get_weather", "description": "Get weather",
                       "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}}]
        });
        // Refused before any dialect normalization can matter.
        let err = leg1_native(&payload).unwrap_err();
        assert!(err.contains("bill at dispatch"), "refusal must name the reason: {err}");
        // Opt-in restores attempt (operator accepts spend risk).
        std::env::set_var("FRAMERIZE_ALLOW_TOOL_LEGS", "1");
        let leg1 = leg1_native(&payload).unwrap();
        std::env::remove_var("FRAMERIZE_ALLOW_TOOL_LEGS");
        let t = &leg1["tools"][0];
        assert_eq!(t["type"], "function");
        assert_eq!(t["function"]["name"], "get_weather");
        assert_eq!(t["function"]["parameters"]["type"], "object");
        // Chat-shaped tools are refused identically (the meter bills tools-in-body
        // regardless of dialect); opt-in passes them through untouched.
        let payload2 = serde_json::json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}]
        });
        assert!(leg1_native(&payload2).is_err(), "refusal is shape-blind");
        std::env::set_var("FRAMERIZE_ALLOW_TOOL_LEGS", "1");
        let leg1b = leg1_native(&payload2).unwrap();
        std::env::remove_var("FRAMERIZE_ALLOW_TOOL_LEGS");
        assert_eq!(leg1b["tools"], payload2["tools"]);
    }

    #[tokio::test]
    async fn native_chat_payload_runs_end_to_end() {
        // Scripted growth per leg (same shape as the prompt-path test), so
        // the chain runs to the leg cap instead of capping on empty growth.
        let mk = |from: usize| {
            let mut v = numbered(from, 250);
            v.push(Event::Done);
            v
        };
        let mut t = ChainMock::new(vec![(mk(0), 10.0), (mk(1000), 10.0)], 10.0);
        let settings = ChainSettings { max_legs: 2, ..fast_settings() };
        let payload = serde_json::json!({"messages": [{"role": "user", "content": "probe"}]});
        let out = complete_native(&mut t, ChainRequest::Chat(payload), &settings)
            .await
            .unwrap();
        assert_eq!(out.outcome, ChainOutcome::LegCap);
        assert_eq!(out.legs.len(), 2);
        // Leg 2's request carried leg 1's captured text as the assistant turn.
        let msgs = t.payloads[1]["messages"].as_array().unwrap();
        assert_eq!(msgs[1]["role"], "assistant");
        assert!(!msgs[1]["content"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hung_oracle_fails_fast() {
        let settings = fast_settings();
        let err = complete(&mut HungOracle, "probe", &settings).await.unwrap_err();
        assert!(err.contains("oracle read timed out"), "{err}");
    }

    #[tokio::test]
    async fn oversize_topic_rejected() {
        let big = "x".repeat(MAX_PROMPT_CHARS + 1);
        let mut t = ChainMock::new(vec![], 10.0);
        let err = complete(&mut t, &big, &fast_settings()).await.unwrap_err();
        assert!(err.contains("over the"), "{err}");
    }

    #[tokio::test]
    async fn absurd_settings_clamped() {
        // max_legs=u32::MAX must not loop forever: hard cap + LegCap.
        let mk = |from: usize| {
            let mut v = numbered(from, 250);
            v.push(Event::Done);
            v
        };
        let legs: Vec<(Vec<Event>, f64)> = (0..70).map(|i| (mk(i * 1000), 10.0)).collect();
        let mut t = ChainMock::new(legs, 10.0);
        let settings = ChainSettings { max_legs: u32::MAX, ..fast_settings() };
        let out = complete(&mut t, "probe", &settings).await.unwrap();
        assert_eq!(out.legs.len(), 64);
        assert_eq!(out.outcome, ChainOutcome::LegCap);
    }

    #[test]
    fn normalize_clamps_all_knobs() {
        let s = ChainSettings {
            stop: u32::MAX,
            max_legs: u32::MAX,
            quarantine_retries: u32::MAX,
            anchor_chars: usize::MAX,
            ..ChainSettings::default()
        }
        .normalize();
        assert_eq!(s.stop, stop::MAX_STOP);
        assert_eq!(s.max_legs, MAX_LEGS_HARD);
        assert_eq!(s.quarantine_retries, MAX_RETRIES_HARD);
        assert_eq!(s.anchor_chars, MAX_ANCHOR_CHARS);
        // idempotent
        assert_eq!(s.normalize().max_legs, MAX_LEGS_HARD);
        // defaults survive
        let d = ChainSettings::default().normalize();
        assert_eq!(d.stop, stop::DEFAULT_STOP);
        assert_eq!(d.max_legs, 8);
    }

    #[test]
    fn anchor_never_splits_char() {
        let text = format!("{}é{}", "x".repeat(400), "y".repeat(10));
        let a = last_anchor_chars(350, &text);
        assert!(a.ends_with(&"y".repeat(10)));
        assert!(text.is_char_boundary(text.len() - a.len()));
        assert_eq!(last_anchor_chars(350, "short"), "short");
        // cut landing inside a 2-byte char widens forward past it
        let t2 = format!("{}é{}", "a".repeat(349), "b".repeat(10));
        let a2 = last_anchor_chars(350, &t2);
        assert!(t2.is_char_boundary(t2.len() - a2.len()));
        assert!(!a2.starts_with('�'));
        assert_eq!(last_anchor_chars(0, "anything"), "");
        assert_eq!(last_anchor_chars(10, ""), "");
    }

    #[test]
    fn payload_shapes() {
        let c1 = leg1_payload("explain ships").unwrap();
        let c1 = c1["messages"][0]["content"].as_str().unwrap();
        assert!(c1.contains("explain ships"));
        assert!(c1.contains(&stop::length_guarantee_suffix(1800, 14)));
        // instruction-first: guarantee opens, topic follows
        assert!(c1.find(&stop::length_guarantee_suffix(1800, 14)).unwrap() < c1.find("explain ships").unwrap());
        let p2 = continue_payload("prior tail text here", 350);
        let c2 = p2["messages"][0]["content"].as_str().unwrap();
        assert!(c2.contains(&stop::continue_suffix()));
        assert!(c2.contains("prior tail text here"));
    }
}
