//! Model registry, from chunk-DM5CYKH3.mjs / chunk-WELZ4GZF.mjs (2026-10-01).
//! The model is NOT sent in the message call — it is chat-session state pinned
//! server-side via the UI's model picker. TODO: capture the session-settings
//! endpoint so compat routes can honor `model`.

/// Model ids as the /ai/v3/* endpoints accept them: NO provider prefix
/// ("gpt-6-sol", not "openai/gpt-6-sol" — the prefixed form 400s with
/// "model pricing is not configured"). The UI's registry uses prefixed ids;
/// strip the prefix when calling the gateway.
pub const DEFAULT_AGENT_MODEL: &str = "gpt-6-sol"; // shown as "GPT 6 Sol"

/// Compatibility models with explicit routing or successful delivery evidence.
/// Arbitrary passthrough names remain accepted but are not advertised here.
pub const COMPAT_MODELS: &[&str] = &[
    "gpt-6-sol", "gpt-6.1-sol", "gpt-6-luna", "gpt-5.6-terra", "gpt-6-astra",
    "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5",
    "gpt-4.1", "gpt-4.1-mini", "gemini-2.5-flash", "gemini-3-flash-preview",
    "moonshotai/kimi-k3:nitro",
];

#[allow(dead_code)]
pub const AGENT_MODELS: &[&str] = &[
    "openai/gpt-6-sol",
    "openai/gpt-5.6-sol",
    "openai/gpt-5.6-terra",
    "openai/gpt-5.6-luna",
    "openai/gpt-5.5",
    "openai/gpt-5.4",
    "openai/gpt-5.3-codex",
    "openai/gpt-5.2-codex",
    "openai/gpt-5.2",
    "openai/gpt-5.1",
    "openai/gpt-4.1",
    "google/gemini-3.8-flash",
    "google/gemini-3.7-flash",
    "google/gemini-3.5-flash",
    "google/gemini-3.1-flash-lite-preview",
    "google/gemini-2.5-flash",
    "google/gemini-2.5-flash-lite",
];

/// Side tasks (session titles, compaction, name suggestions) run on cheap Gemini.
#[allow(dead_code)]
pub const SIDE_TASK_MODEL: &str = "google/gemini-3-flash-preview";
#[allow(dead_code)]
pub const IMAGE_MODEL: &str = "gpt-image-2.5-flare";
