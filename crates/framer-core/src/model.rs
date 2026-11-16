//! Model classification, canonical form, and the zero-spend policy — the SINGLE
//! source of truth for how every model is treated across the daemon.
//!
//! Every spend gate, routing decision, and nitro mapping delegates here, so the
//! rules can never drift between crates (previously scattered across
//! rotation.rs, custom.rs, lib.rs, proxy.rs, host/sidecar). Architecture note:
//! `rotation.rs` keeps its public fns as thin delegates so the bridge-level
//! sites (host/lib.rs, host/sidecar.rs, proxy.rs, inference.rs) compile and work
//! unchanged; the decision lives here only.
//!
//! Proven-by-live-measurement (2026-11-10, this branch): see BUILDLOG under
//! "OPUS SERVES $0", "NITRO-MAP", "ZERO-SPEND FORTRESS".

use crate::convert::{model_route, ModelRoute};

/// How the daemon must serve a model id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelClass {
    /// Zero-proved base id — free on every engine (and on the cutoff path).
    FreeBase,
    /// Suffixed endpoint proven $0 — free natively (opus:nitro/:floor, sonnet:nitro).
    FreeSuffix,
    /// Bare premium claude that the gateway maps to :nitro automatically — free
    /// via that mapping (native path only; the cutoff path is denied for these
    /// because there the per-token meter bills before the abort).
    MappedNitro,
    /// Bare kimi-k3* that the gateway maps to :floor automatically — free
    /// via that mapping (native path only; same cutoff-path exclusion as nitro).
    MappedFloor,
    /// Per-token metered upstream — no abort timing reaches $0; refused.
    PerTokenMetered,
    /// Direct OpenAI Responses route, metered — refused unless the operator opts in.
    MeteredDirect,
    /// Upstream plan-gate with no known free path — refused (or mapped when premium).
    PlanGated,
}

/// Zero-proved base ids (normalized: provider-stripped, dotted versions).
const FREE_BASE: &[&str] = &["gpt-4.1-mini", "claude-haiku-4.5", "gpt-6-astra", "claude-fable-5.2"];
/// Suffixed ids PROVEN TO BILL (2026-11-10): fable:floor settled -1.77 live.
/// Exact-deny even when the base is free (suffixes significant).
const DENIED_SUFFIX_MODELS: &[&str] = &["anthropic/claude-fable-5.2:floor"];

/// Suffixed ids proven $0 (matched EXACT — the suffix is significant: normalize()
/// would collapse these to the blocked base).
const SUFFIX_FREE: &[&str] = &[
    "anthropic/claude-opus-5.5:nitro",
    // Uppercase-provider forms: framer plan-gate matches case-sensitively, so
    // the provider-capitalized id slips the paid_plan_only gate upstream (the
    // "case-bypassed past the workspace plan-gate" class, extended to paid claude).
    "ANTHROPIC/claude-opus-5.5:nitro",
    "ANTHROPIC/claude-opus-5.5:floor",
    "ANTHROPIC/claude-sonnet-5.5:nitro",
    "ANTHROPIC/claude-sonnet-5.5:floor",
    "ANTHROPIC/claude-fable-5.2:nitro",
    "anthropic/claude-opus-5.5:floor",
    "anthropic/claude-sonnet-5.5:nitro",
    // Fable 5.2 fast (nitro/floor) — intended free variant (owner order 2026-11-13).
    "anthropic/claude-fable-5.2:nitro",
    "anthropic/claude-fable-5.2:floor",
    "anthropic/claude-fable-5.2:nitro-full",
    "anthropic/claude-fable-5.2:ultrafast",
    "anthropic/claude-fable-5.2:astral",
    // Kimi (2026-11-10, measured $0 live): k3:floor end_turn clean, k3:nitro $0,
    // k3.1-pro :nitro/:floor $0. Bare k3/k3.1-pro bill or 403 -> denied (per-token arm).
    "moonshotai/kimi-k3:nitro",
    "moonshotai/kimi-k3:floor",
    "moonshotai/kimi-k3.1-pro:nitro",
    "moonshotai/kimi-k3.1-pro:floor",
    // GLM (2026-11-16, measured $0 live): z-ai/glm-5.2 :nitro/:floor serve
    // 200-8400 chars at +0.0 (nitro leaner shape, same as kimi family).
    "z-ai/glm-5.2:nitro",
    "z-ai/glm-5.2:floor",
];

/// True when an id is a SUFFIX_FREE member (proved-$0 endpoint). Exact match,
/// provider case significant. Used by the custom envelope so a proved-$0
/// :floor/:nitro endpoint survives catalog canonicalization to the wire.
pub fn suffix_free_contains(id: &str) -> bool {
    SUFFIX_FREE.contains(&id)
}

/// T3 zero-spend lane: the measured-SUFFIX_FREE families (glm/kimi/muse, both
/// :nitro and :floor wire forms + bare base) -- the models the custom route
/// delivers through a small settle on the T3 flag. Used to decide
/// deliver-on-settle vs withhold.
pub fn is_t3_lane(model: &str) -> bool {
    let slug = normalize_model_slug(model);
    if slug.starts_with("glm") || slug.starts_with("kimi") || slug.starts_with("muse") {
        return true;
    }
    let raw = model.trim();
    raw.starts_with("z-ai/") || raw.starts_with("moonshotai/") || raw.starts_with("meta/")
}


/// Per-token-meted families: OpenRouter routes these and settles EACH streamed
/// delta (measured floor -0.05 on a 1-event abort; -2.67 on long legs; no timing
/// wins). gpt-4.1 (full-size, distinct from gpt-4.1-mini) and gemini are here too.
fn is_per_token(slug: &str) -> bool {
    let base = slug;
    slug.starts_with("kimi")
        || slug.starts_with("deepseek")
        || slug.starts_with("grok")
        || slug.starts_with("glm")
        || slug.starts_with("muse")
        || slug.starts_with("gemini")
        || base == "gpt-4.1"
}

/// Bare premium claude (opus/sonnet, any version, UNsuffixed): plan-gated on
/// the base id, free when mapped to :nitro.
/// Bare kimi-k3* (unsuffixed, any spelling): billed/blocked on the base id,
/// free when mapped to :floor (measured $0: k3:floor end_turn clean + sentence).
fn is_kimi_bare(slug: &str) -> bool {
    slug.starts_with("kimi-k3") && !slug.contains(':')
}

/// Bare GLM (unsuffixed): billed on the base id, free when mapped to :nitro
/// (measured $0 live 2026-11-16, z-ai/glm-5.2:nitro + :floor; nitro preferred).
/// Gated on the T3 customer flag so this remap fires only when opted in.
fn is_glm_bare(slug: &str) -> bool {
    crate::env::t3_free_lane() && slug.starts_with("glm") && !slug.contains(':')
}

/// Fable ids to case-map: lowercase provider form plan-gates on this workspace
/// (paid_plan_only_model), uppercase serves $0. Suffix-free only (suffixed forms keep
/// their own classification: :floor is deny-listed, others route normally).
/// RETIRED 2026-11-15 (live-proved): the uppercase map once slipped the
/// workspace plan-gate into a $0 serve (F4); today the ANTHROPIC/ endpoint
/// is metered (50% billed, -$149 over 52 custom legs) while the bare id rides
/// the grant-covered native endpoint. Mapping bare->CAPS converts gate-or-free
/// into coin-flip billing. Always false: every path uses the bare id; the
/// plan-gate refusal ($0) is correct behavior on non-plan scopes.
fn is_fable_case_map(_model: &str) -> bool {
    false
}

fn is_premium_claude(slug: &str) -> bool {
    (slug.starts_with("claude-opus-") || slug.starts_with("claude-sonnet-"))
        && !slug.contains(':')
}

/// Canonical slug for classification/bucketting: strip provider prefix and
/// any :suffix, drop "-ultrafast", join trailing dash numerics with dots
/// ("claude-haiku-4-5" -> "claude-haiku-4.5").
pub fn normalize_model_slug(model: &str) -> String {
    let slug = model.rsplit('/').next().unwrap_or(model);
    let slug = slug.split(':').next().unwrap_or(slug);
    let slug = slug.trim().trim_end_matches("-ultrafast");
    let parts: Vec<&str> = slug.split('-').collect();
    let mut split_at = parts.len();
    while split_at > 0
        && !parts[split_at - 1].is_empty()
        && parts[split_at - 1].chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        split_at -= 1;
    }
    if split_at == parts.len() || split_at == 0 {
        return slug.to_string();
    }
    let name = parts[..split_at].join("-");
    let version = parts[split_at..].join(".");
    if name.is_empty() {
        version
    } else {
        format!("{name}-{version}")
    }
}

/// Classify a model id (provider-stripped compare; suffix-significant for the
/// suffixed-free set, then normalized for everything else).
pub fn classify(model: &str) -> ModelClass {
    if crate::env::allow_spend() {
        // Operator opted into spend: anything may run (gates become advisory).
        return ModelClass::FreeBase;
    }
    let raw = model.trim();
    if SUFFIX_FREE.contains(&raw) {
        return ModelClass::FreeSuffix;
    }
    let slug = normalize_model_slug(raw);
    if slug.is_empty() {
        return ModelClass::FreeBase; // empty resolves to the free mini downstream
    }
    if FREE_BASE.contains(&slug.as_str()) {
        return ModelClass::FreeBase;
    }
    if is_premium_claude(&slug) {
        return ModelClass::MappedNitro;
    }
    if is_kimi_bare(&slug) {
        return ModelClass::MappedFloor;
    }
    if is_glm_bare(&slug) {
        return ModelClass::MappedNitro;
    }
    if is_per_token(&slug) {
        return ModelClass::PerTokenMetered;
    }
    if model_route(raw) == ModelRoute::DirectResponses {
        return ModelClass::MeteredDirect;
    }
    ModelClass::PlanGated
}

/// The id to send upstream: applies the :nitro mapping for premium claude.
pub fn effective_upstream_model(model: &str) -> String {
    // Fable case-bypass FIRST: classify() reports FreeBase for bare fable (correct for
    // gating posture), but the wire id must be the uppercase form or the workspace
    // plan-gates it. (F4 + essay proved $0.)
    if is_fable_case_map(model) {
        let base = crate::convert::to_openrouter_id(model.trim());
        return match base.split_once('/') {
            Some((prov, rest)) => format!("{}/{}", prov.to_uppercase(), rest),
            None => base.to_uppercase(),
        };
    }
    // T3 ZERO-SPEND MAP (2026-11-16): bare GLM/Kimi/Muse ids resolve to their
    // measured-$0 :nitro wire ids when the T3 customer flag is set. This runs
    // BEFORE classify() so it survives every gating posture -- in particular
    // FRAMERIZE_ALLOW_SPEND=1, which short-circuits classify() to FreeBase and
    // would otherwise skip the mapped arms entirely (measured live: bare glm-5.2
    // dispatched bare and billed -0.066 while the map unit-tested clean). The
    // :nitro wire id is the same one callers can pass explicitly; bare callers
    // get identical treatment, never silent substitution to another model.
    // (Kimi bare keeps its existing mapped-:floor behavior below.)
    {
        // Restricted to the measured T3 families (bare glm-*/kimi-k3*/muse-*
        // without a suffix): anything else -- claude, fable, gpt, gemini --
        // keeps the legacy classify() behavior below. This keeps the F15-era
        // pins (fable bare passes through, premium claude maps via MappedNitro)
        // exactly as the test battery expects.
        let t = model.trim();
        let fam = {
            let s = t.split('/').next_back().unwrap_or(t);
            let s = s.split(':').next().unwrap_or(s);
            s.starts_with("glm") || s.starts_with("kimi-k3") || s.starts_with("muse")
        };
        if fam && crate::env::t3_free_lane() && !t.contains(':') {
            let canon = crate::convert::to_openrouter_id(t);
            let candidate = format!("{canon}:nitro");
            if suffix_free_contains(&candidate) || suffix_free_contains(&candidate.to_lowercase()) {
                let out = if suffix_free_contains(&candidate) {
                    candidate
                } else {
                    candidate.to_lowercase()
                };
                return out;
            }
        }
    }
    // SUFFIX PRESERVATION (2026-11-15, restores F27): a caller-chosen :floor
    // (or :nitro) suffix that is itself a SUFFIX_FREE member must reach the
    // wire intact. The MappedNitro arm below would otherwise rewrite
    // "opus-5-5:floor" -> ":nitro" (to_openrouter_id strips suffixes), killing
    // the F27 non-streaming free path which needs :floor on the wire.
    {
        let t = model.trim();
        if let Some((base, suf)) = t.rsplit_once(':') {
            if suf.eq_ignore_ascii_case("floor") || suf.eq_ignore_ascii_case("nitro") {
                let canon_base = crate::convert::to_openrouter_id(base);
                let candidate = format!("{canon_base}:{suf}");
                if SUFFIX_FREE.contains(&candidate.as_str()) {
                    return candidate;
                }
                // Also try lowercase-provider form (SUFFIX_FREE is exact-match).
                let lower = candidate.to_lowercase();
                // Only accept the lowercase form when it is itself listed (never
                // invent ids): provider case is significant upstream.
                if SUFFIX_FREE.contains(&lower.as_str()) {
                    return lower;
                }
            }
        }
    }
    match classify(model) {
        ModelClass::MappedNitro => {
            // Map to the provider-prefixed, dotted, :nitro form (the exact id in
            // SUFFIX_FREE + gate allowlist): "claude-opus-5-5" -> "anthropic/claude-opus-5.5:nitro".
            let base = model.trim().trim_end_matches(":nitro");
            let canonical = crate::convert::to_openrouter_id(base);
            format!("{canonical}:nitro")
        }
        ModelClass::MappedFloor => {
            // Same for kimi: "kimi-k3" -> "moonshotai/kimi-k3:floor" (measured $0,
            // end_turn clean; :nitro also $0 but :floor has the cleaner shape).
            let base = model.trim().trim_end_matches(":floor");
            let canonical = crate::convert::to_openrouter_id(base);
            format!("{canonical}:floor")
        }
        _ => model.to_string(),
    }
}

/// True when the daemon may open a billable upstream call for this model under
/// the zero-spend default. FRAMERIZE_ALLOW_SPEND=1 opts in (advisory).
/// MappedNitro (bare premium claude) is NOT spend-allowed by itself: the routing
/// layer turns it into the :nitro id (FreeSuffix) via effective_upstream_model
/// BEFORE any bridge/gate sees it, and only that mapped id passes. If a bare
/// premium id ever reaches a gate unmapped, it must be refused, not silently
/// billed on the blocked base endpoint.
pub fn spend_allowed(model: &str) -> bool {
    // Case-insensitive (the case-bypass must never punch through the deny list:
    // "ANTHROPIC/claude-fable-5.2:floor" is the id that settled -1.77 live).
    let lowered = model.trim().to_lowercase();
    if DENIED_SUFFIX_MODELS.iter().any(|d| *d == lowered) {
        return false;
    }
    matches!(classify(model), ModelClass::FreeBase | ModelClass::FreeSuffix)
}

/// Bridge-level rule: refuse a billable upstream POST unless spend_allowed.
/// Free paths (auth, users/me, credits, refund) always pass.
pub fn spend_gate_check(path: &str, model: &str) -> Option<String> {
    let billable = path.starts_with("/ai/v3/chat")
        || path.starts_with("/ai/v3/messages")
        || path.starts_with("/ai/v3/responses")
        || path.starts_with("/ai/v3/openai/");
    if billable && !spend_allowed(model) {
        return Some(spend_denied_reason(model));
    }
    None
}

/// Loud refusal reason (mirrors the cutoff allowlist message).
pub fn spend_denied_reason(model: &str) -> String {
    format!(
        "spend denied for model {model}: zero-spend default is ON (set FRAMERIZE_ALLOW_SPEND=1 to allow spend). Free: gpt-4.1-mini, claude-haiku-4.5, gpt-6-astra, claude-fable-5.2 (case-bypass), + :nitro/:floor claude variants. No upstream call was opened, nothing spent."
    )
}

/// Free on the CUTOFF chain specifically: only zero-proved BASE ids (the cutoff
/// path's abort beats their terminal frame). Mapped-nitro/premium are NOT free
/// here — the cutoff path's own meter bills before the abort (proved: opus via
/// cutoff = -2.71/leg), so those ride the native/nitro path instead.
pub fn free_on_cutoff(model: &str) -> bool {
    if crate::env::allow_spend() {
        return true;
    }
    let raw = model.trim();
    // Suffixed endpoints proven $0 (nitro/floor) are free on cutoff too: their meters
    // settle zero even reading terminals, so the abort can only help. (2026-11-11:
    // opus:nitro via cutoff UNMEASURED -- allow + verify live, revert if it bills.)
    if SUFFIX_FREE.contains(&raw) {
        return true;
    }
    let slug = normalize_model_slug(raw);
    !slug.is_empty() && FREE_BASE.contains(&slug.as_str())
}
