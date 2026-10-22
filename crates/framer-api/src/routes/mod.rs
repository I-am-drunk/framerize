//! Upstream route registry (2026-10-06).
//!
//! Every Framer AI route this gateway speaks lives in ONE module under
//! `routes/`. Each route owns its path, dialect, metering behavior, and the
//! cheap probe used by the drift canary. Everything that needs a route
//! resolves it here — never hardcode an `/ai/v3/...` literal elsewhere.
//!
//! Deleting a route = delete its file + remove its entry in `ALL`. The
//! compiler then points at every remaining user, so dead routes can't linger
//! as invisible string literals.

pub mod direct_anthropic;
pub mod direct_responses;
pub mod fetch;
pub mod openrouter_chat;
pub mod openrouter_messages;
pub mod openrouter_responses;
pub mod refund;

use crate::proxy::Dialect;

/// How a route bills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metering {
    /// Non-streaming requests bypass the meter (the free-route gap); SSE is
    /// metered. The standing exploit — drift canary alarms if this flips.
    FreeNonStreaming,
    /// Every request is metered (streaming or not).
    AlwaysMetered,
    /// No metering at all (utility routes).
    Unmetered,
}

/// One upstream route.
pub struct Route {
    /// Stable id used in logs, events, and the drift canary ("openrouter-messages").
    pub id: &'static str,
    /// The upstream path ("/ai/v3/messages/").
    pub path: &'static str,
    /// The proxy dialect that drives it.
    pub dialect: Dialect,
    pub metering: Metering,
    /// Cheap model id (as sent upstream) for drift probes.
    pub probe_model: &'static str,
    /// True if this route carries production inference traffic (drift canary
    /// probes these every cycle).
    pub primary: bool,
}

/// The registry. Order is the chain's preference order for bucket rotation.
pub const ALL: &[&Route] = &[
    &openrouter_messages::ROUTE,
    &openrouter_chat::ROUTE,
    &openrouter_responses::ROUTE,
    &direct_responses::ROUTE,
    &direct_anthropic::ROUTE,
    &refund::ROUTE,
    &fetch::ROUTE,
];

pub fn by_dialect(d: Dialect) -> &'static Route {
    ALL.iter().copied().find(|r| std::mem::discriminant(&r.dialect) == std::mem::discriminant(&d))
        .expect("every proxy dialect has a route")
}

pub fn by_id(id: &str) -> Option<&'static Route> {
    ALL.iter().copied().find(|r| r.id == id)
}

/// Routes the drift canary probes every cycle (primary inference carriers).
pub fn primary() -> impl Iterator<Item = &'static Route> {
    ALL.iter().copied().filter(|r| r.primary)
}

/// Supported inference routes, independently of drift-canary selection.
pub fn inference() -> impl Iterator<Item = &'static Route> {
    [
        &openrouter_messages::ROUTE,
        &openrouter_chat::ROUTE,
        &openrouter_responses::ROUTE,
        &direct_responses::ROUTE,
        &direct_anthropic::ROUTE,
    ].into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dialect_has_a_route() {
        for d in [
            Dialect::OpenRouterMessages,
            Dialect::OpenRouterChat,
            Dialect::OpenAiResponses,
            Dialect::AnthropicMessages,
        ] {
            let r = by_dialect(d);
            assert_eq!(std::mem::discriminant(&r.dialect), std::mem::discriminant(&d));
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = ALL.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len());
    }

    #[test]
    fn free_routes_are_the_openrouter_passthroughs() {
        let free: Vec<_> = ALL.iter().filter(|r| r.metering == Metering::FreeNonStreaming).map(|r| r.id).collect();
        assert_eq!(free, vec!["openrouter-messages", "openrouter-chat", "openrouter-responses"]);
    }
}
