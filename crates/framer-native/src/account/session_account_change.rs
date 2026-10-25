//! Account-first lifecycle ownership for login, removal and browser changes.
use super::*;
use std::panic::AssertUnwindSafe;

/// Issued only after the owned job has dropped its native mutation claims.
/// Finished errors keep closed, retryable barriers; Unconfirmed poisons them.
pub enum AccountChangeResult<T> {
    NotStarted(String),
    Finished(Result<T, String>),
    Unconfirmed(String),
}
impl<T> AccountChangeResult<T> {
    pub fn into_result(self) -> Result<T, String> {
        match self {
            Self::Finished(value) => value,
            Self::NotStarted(error) | Self::Unconfirmed(error) => Err(error),
        }
    }
}

struct AccountMutation {
    native: NativeAccountClient,
    ticket: AccountChange,
    uncertain: bool,
}

impl SessionStoreBridge {
    /// Fence AccountId before browser mutation, including unknown-user login.
    /// An admitted operation survives waiter loss. Include private save,
    /// StateStore publication and actual cleanup in this closure.
    pub async fn change_account<T, F>(&self, account: AccountId, update: F) -> Result<T, String>
    where T: Send + 'static,
        F: for<'a> FnOnce(&'a mut ChangingAccount) -> BoxFuture<'a, ChangeOutcome<T>> + Send + 'static,
    {
        self.change_account_observed(account, update).await.into_result()
    }

    /// Keep external ownership until this result, including on failure.
    pub async fn change_account_observed<T, F>(&self, account: AccountId, update: F)
        -> AccountChangeResult<T>
    where T: Send + 'static,
        F: for<'a> FnOnce(&'a mut ChangingAccount) -> BoxFuture<'a, ChangeOutcome<T>> + Send + 'static,
    {
        let native = self.inner.native.clone();
        let store = self.inner.store.clone();
        let gate = self.inner.admission.clone();
        let mut owns_barrier = false;
        let receiver = self.submit(|admission| {
            let (stamp, ticket) = admission.account_change(&native, account)?;
            owns_barrier = true;
            let mutation = AccountMutation { native: native.clone(), ticket, uncertain: false };
            let access = ChangingAccount { account, native: native.clone(), store,
                admission: gate.clone(), session: None, session_failed: false };
            Ok(async move {
                Ok(mutation.run(access, stamp, gate, update).await)
            })
        });
        let receiver = match receiver {
            Ok(receiver) => receiver,
            Err(error) if !owns_barrier => return AccountChangeResult::NotStarted(error),
            Err(error) => return AccountChangeResult::Unconfirmed(error),
        };
        match receiver.await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => AccountChangeResult::Unconfirmed(error),
            Err(_) => AccountChangeResult::Unconfirmed(STOPPED.to_owned()),
        }
    }
}
impl Drop for AccountMutation {
    fn drop(&mut self) {
        if self.uncertain { let _ = self.native.fail_account_change(&self.ticket); }
    }
}

impl AccountMutation {
    async fn apply<T, F>(&mut self, access: &mut ChangingAccount, stamp: &Stamp,
        gate: &Mutex<Admission>, update: F) -> AccountChangeResult<T>
    where T: Send + 'static,
        F: for<'a> FnOnce(&'a mut ChangingAccount) -> BoxFuture<'a, ChangeOutcome<T>> + Send + 'static,
    {
        if self.ticket.ready().await.is_err() {
            return AccountChangeResult::Unconfirmed("account cleanup unconfirmed".to_owned());
        }
        self.uncertain = true;
        let outcome = update(access).await;
        let incomplete = access.session.as_ref().is_some_and(|held| !held.ready);
        let uncertain_session = access.session.as_ref()
            .is_some_and(|held| !held.ready && held.mutation.uncertain);
        self.uncertain = !outcome.cleanup_confirmed || uncertain_session;
        if let Some(held) = &mut access.session { held.mutation.uncertain = self.uncertain; }
        if self.uncertain {
            return AccountChangeResult::Unconfirmed("account cleanup unconfirmed".to_owned());
        }
        // Dropped preparation may still be draining old native work.
        if let Some(held) = &mut access.session {
            if held.mutation.ticket.ready().await.is_err() {
                self.uncertain = true;
                held.mutation.uncertain = true;
                return AccountChangeResult::Unconfirmed("session cleanup unconfirmed".to_owned());
            }
        }
        let value = match outcome.value {
            Ok(value) => value,
            Err(error) => return AccountChangeResult::Finished(Err(error)),
        };
        if access.session_failed || incomplete {
            return AccountChangeResult::Finished(Err("account session binding failed".to_owned()));
        }
        let mut admission = lock(gate);
        if self.native.finish_account_and_session_change(&mut self.ticket,
            access.session.as_mut().map(|held| &mut held.mutation.ticket)).is_err() {
            self.uncertain = true;
            if let Some(held) = &mut access.session { held.mutation.uncertain = true; }
            return AccountChangeResult::Unconfirmed("account change completion failed".to_owned());
        }
        admission.versions.unblock(stamp);
        if let Some(held) = &access.session { admission.versions.unblock(&held.stamp); }
        AccountChangeResult::Finished(Ok(value))
    }

    async fn run<T, F>(mut self, mut access: ChangingAccount, stamp: Stamp,
        gate: Arc<Mutex<Admission>>, update: F) -> AccountChangeResult<T>
    where T: Send + 'static,
        F: for<'a> FnOnce(&'a mut ChangingAccount) -> BoxFuture<'a, ChangeOutcome<T>> + Send + 'static,
    {
        let result = AssertUnwindSafe(self.apply(&mut access, &stamp, &gate, update))
            .catch_unwind().await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                self.uncertain = true;
                if let Some(held) = &mut access.session { held.mutation.uncertain = true; }
                AccountChangeResult::Unconfirmed("account mutation panicked".to_owned())
            }
        };
        // Publish only after claims and their failure guards are relinquished.
        drop(access);
        drop(self);
        result
    }
}

/// Available only after the old native generation closes. The caller must
/// separately own the shared legacy/native account lease.
pub struct ChangingAccount {
    account: AccountId,
    native: NativeAccountClient,
    store: Arc<dyn SessionStore>,
    admission: Arc<Mutex<Admission>>,
    session: Option<UserMutation>,
    session_failed: bool,
}
impl ChangingAccount {
    pub fn account(&self) -> AccountId { self.account }

    /// Fence the discovered user before private save and StateStore upsert.
    /// One target is allowed; its fence lasts through the outer change.
    /// This uses the current actor job, without waiting for another slot.
    pub async fn session(&mut self, user: FramerUserId) -> Result<&ChangingSession, String> {
        let result = self.prepare_session(user).await;
        if result.is_err() { self.session_failed = true; }
        result?;
        Ok(&self.session.as_ref().expect("prepared session").access)
    }

    async fn prepare_session(&mut self, user: FramerUserId) -> Result<(), String> {
        if let Some(held) = &self.session {
            return if held.access.user == user && held.ready {
                Ok(())
            } else { Err("account change already bound to a session".to_owned()) };
        }
        let held = UserMutation::new(&mut lock(&self.admission), &self.native,
            self.store.clone(), user)?;
        self.session = Some(held);
        self.session.as_mut().expect("owned session").initialize().await
    }
}
