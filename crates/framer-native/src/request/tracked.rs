use super::*;
pub(super) struct TrackedAuthorization {
    pub(super) inner: Arc<dyn AttemptAuthorization>, pub(super) registry: Arc<lifetime::Registry>,
}
impl TrackedAuthorization {
    pub(super) fn error(&self, error: crate::request::invocation::ApplicationError) -> DispatchError {
        use crate::request::invocation::ApplicationError;
        use crate::account::client::ClientError;
        use crate::account::owner::AdmissionError;
        fn needs_account_cleanup(error: &ApplicationError) -> bool {
            match error {
                ApplicationError::Start(ClientError::Draining | ClientError::Attachment
                    | ClientError::Admission(AdmissionError::SpawnFailed)) => true,
                ApplicationError::Chain { error, .. } => needs_account_cleanup(error),
                _ => false,
            }
        }
        // Keep this evidence before mapping away the native start-error type.
        if needs_account_cleanup(&error) { self.registry.require_account_cleanup(); }
        errors::error(error)
    }
}
impl AttemptAuthorization for TrackedAuthorization {
    fn observe_finish(&self, metadata: &crate::request::invocation::AttemptMetadata) {
        self.registry.completed(errors::metadata(metadata.clone()));
        self.inner.observe_finish(metadata);
    }
    fn authorize<'a>(&'a self, attempt: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        async move {
            let observer = |cleanup, cancellation| self.registry.preparing(cleanup, cancellation);
            self.inner.authorize_observed(attempt, &observer).await
        }.boxed()
    }
    fn observe_start(&self,
        cleanup: crate::transport::downstream::Cleanup,
        cancellation: crate::account::owner::Cancellation) {
        self.registry.started(cleanup.clone(), cancellation.clone());
        self.inner.observe_start(cleanup, cancellation);
    }
}
