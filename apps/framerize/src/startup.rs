use framer_server::AppState;

pub(crate) fn start_prewarm(state: &AppState) -> Result<Option<tokio::task::JoinHandle<()>>, String> {
    let prewarm_state = state.clone();
    framer_server::operation_ownership::spawn(state.operation_ownership.as_ref(),
        framer_core::operation_gate::OperationKind::Background,
        async move { framer_server::prewarm_accounts(prewarm_state).await },
    ).map(Some).map_err(|error| {
        let cause = state.operation_ownership.as_ref().map(|scope| scope.admission_message(error))
            .unwrap_or_else(|| error.explanation().to_owned());
        format!("startup ownership unavailable: {cause}")
    })
}
