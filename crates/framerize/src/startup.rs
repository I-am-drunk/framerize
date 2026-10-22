use framer_server::AppState;

pub(crate) fn start_prewarm(state: &AppState) -> Result<Option<tokio::task::JoinHandle<()>>, String> {
    // Native inference admits lazily. Explicit browser administration remains available.
    #[cfg(feature = "native-dispatch")]
    if state.native_dispatch.is_some() { return Ok(None); }
    let prewarm_state = state.clone();
    framer_server::operation_ownership::spawn(state.operation_ownership.as_ref(),
        framer_core::operation_gate::OperationKind::Background,
        async move { framer_server::prewarm_accounts(prewarm_state).await },
    ).map(Some).map_err(|error| format!("startup ownership unavailable: {error:?}"))
}
