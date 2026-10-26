//! Account-bound HTTP reuse. A request keeps its generation until cleanup;
//! idle generations are bounded and remain subject to the mutation barriers.
use super::*;
use tokio::time::{Duration, Instant};

const IDLE_TTL: Duration = Duration::from_secs(15);

type LeaseDelivery = oneshot::Receiver<Result<Result<Versioned<AccountLease>, OpenAccountError>, String>>;

impl SessionStoreBridge {
    pub(crate) async fn lease_account_observed(&self, account: AccountId, user: FramerUserId,
        limits: NativeHttpLimits, egress: NativeEgress<'_>) -> Result<AccountLease, OpenAccountError> {
        validate_user(&user)?;
        let proxy = match egress {
            NativeEgress::Direct => None, NativeEgress::Proxy(p) => Some(p.to_owned()),
        };
        let native = self.inner.native.clone();
        let store = self.inner.store.clone();
        let gate = self.inner.admission.clone();
        let request = LeaseRequest { account, user, limits, proxy };
        let receiver = self.submit(move |admission| {
            let user = admission.versions.read(Key::User(request.user.clone()))?;
            let account = admission.versions.read(Key::Account(request.account))?;
            Ok(async move { Ok(request.open(native, store, gate, user, account).await) })
        })?;
        self.publish_lease(receiver).await
    }

    async fn publish_lease(&self, receiver: LeaseDelivery) -> Result<AccountLease, OpenAccountError> {
        let result = match receiver.await {
            Ok(result) => result??,
            Err(_) => {
                let _ = self.inner.closed.clone().await;
                return Err(OpenAccountError::CleanupUnconfirmed);
            }
        };
        if let Err(error) = result.validate(&self.inner.admission) {
            let closed = result.value.finish(false).await;
            let stopping = lock(&self.inner.admission).sender.is_none();
            let joined = !stopping || self.inner.closed.clone().await.is_ok();
            return Err(if closed && joined { OpenAccountError::Rejected(error) }
                else { OpenAccountError::CleanupUnconfirmed });
        }
        Ok(result.value)
    }
}

pub(super) struct Retained {
    native: NativeAccountClient,
    scope: AccountScope,
    session: Session,
    limits: NativeHttpLimits,
    proxy: Option<String>,
    last_used: Mutex<Instant>,
}

impl Retained {
    fn route(&self) -> NativeEgress<'_> {
        self.proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct)
    }
    pub(super) async fn close(&self) -> bool {
        match self.native.close_account(&self.scope) {
            Ok(closed) => closed.await.is_ok(), Err(_) => false,
        }
    }
}

struct LeaseRequest {
    account: AccountId,
    user: FramerUserId,
    limits: NativeHttpLimits,
    proxy: Option<String>,
}

impl LeaseRequest {
    async fn open_loaded(&self, native: NativeAccountClient, store: Arc<dyn SessionStore>,
        gate: Arc<Mutex<Admission>>, session: Session, user: Stamp, account: Stamp)
        -> Result<Versioned<AccountLease>, OpenAccountError> {
        for _ in 0..=CAPACITY {
            let choice = {
                let mut current = lock(&gate);
                if !current.versions.current(&user) || !current.versions.current(&account) {
                    return Err(CHANGED.to_owned().into());
                }
                if current.sender.is_none() { return Err(STOPPED.to_owned().into()); }
                self.choose_or_open(&native, &store, &mut current, &session)?
            };
            match choice {
                Choice::Ready(value) => return Ok(Versioned { value, user, account: Some(account) }),
                Choice::Close(owner) if !owner.close().await => {
                    return Err(OpenAccountError::CleanupUnconfirmed);
                }
                Choice::Close(_) => {},
                Choice::Open => unreachable!("open choice is resolved under admission"),
            }
        }
        Err("native account lease changed repeatedly".to_owned().into())
    }

    async fn open(self, native: NativeAccountClient, store: Arc<dyn SessionStore>,
        gate: Arc<Mutex<Admission>>, user: Stamp, account: Stamp)
        -> Result<Versioned<AccountLease>, OpenAccountError> {
        let session = store.load(&self.user).await.map_err(|_| "session load failed".to_owned())?
            .ok_or_else(|| "session not found".to_owned())?;
        if session.active_user != self.user {
            return Err("session identity mismatch".to_owned().into());
        }
        self.open_loaded(native, store, gate, session, user, account).await
    }

    fn choose_or_open(&self, native: &NativeAccountClient, store: &Arc<dyn SessionStore>,
        current: &mut Admission, session: &Session) -> Result<Choice, String> {
        match current.leases.choose(self.account, session, self.limits, &self.proxy)? {
            Choice::Open => {
                let route = self.proxy.as_deref().map(NativeEgress::Proxy)
                    .unwrap_or(NativeEgress::Direct);
                let scope = native.open_account_owned(self.account, session.clone(),
                    self.limits, route, Some(store.clone()))
                    .map_err(|_| "native account open failed".to_owned())?;
                Ok(Choice::Ready(current.leases.insert(native.clone(), scope,
                    session.clone(), self.limits, self.proxy.clone())))
            }
            choice => Ok(choice),
        }
    }
}
impl Drop for Retained {
    fn drop(&mut self) { let _ = self.native.close_account(&self.scope); }
}

/// Request ownership is independent of the idle cache's ownership.
pub(crate) struct AccountLease { owner: Arc<Retained>, reusable: bool }
impl AccountLease {
    pub(crate) fn scope(&self) -> &AccountScope { &self.owner.scope }
    pub(crate) async fn finish(mut self, reusable: bool) -> bool {
        if reusable && self.owner.scope.matches_egress(&self.owner.route()) {
            self.reusable = true;
            true
        }
        else { self.owner.close().await }
    }
}
impl Drop for AccountLease {
    fn drop(&mut self) {
        if !self.reusable { let _ = self.owner.native.close_account(&self.owner.scope); }
        *lock(&self.owner.last_used) = Instant::now();
    }
}

#[derive(Default)]
pub(super) struct LeaseCache { owners: Vec<Arc<Retained>> }
enum Choice { Ready(AccountLease), Close(Arc<Retained>), Open }

impl LeaseCache {
    pub(super) fn close_all(&self) {
        for owner in &self.owners { let _ = owner.native.close_account(&owner.scope); }
    }
    pub(super) fn take(&mut self) -> Vec<Arc<Retained>> { std::mem::take(&mut self.owners) }
    fn prune(&mut self) {
        self.owners.retain(|owner| owner.scope.closed().now_or_never().is_none());
    }

    pub(super) fn expire(&mut self) {
        for owner in &self.owners {
            if Arc::strong_count(owner) == 1 && lock(&owner.last_used).elapsed() >= IDLE_TTL {
                let _ = owner.native.close_account(&owner.scope);
            }
        }
        self.prune();
    }

    fn choose(&mut self, account: AccountId, session: &Session,
        limits: NativeHttpLimits, proxy: &Option<String>) -> Result<Choice, String> {
        self.prune();
        if let Some(index) = self.owners.iter().position(|owner| owner.scope.account() == account) {
            let owner = &self.owners[index];
            if !owner.scope.matches_egress(&owner.route()) {
                return Ok(self.retire(index));
            }
            if owner.session.active_user == session.active_user
                && owner.session.cookie_header == session.cookie_header
                && owner.limits == limits && &owner.proxy == proxy
                && owner.scope.matches_egress(&owner.route()) {
                return Ok(Choice::Ready(AccountLease { owner: owner.clone(), reusable: false }));
            }
            if Arc::strong_count(owner) != 1 {
                return Err("native account transport still in use".to_owned());
            }
            return Ok(self.retire(index));
        }
        self.make_room()
    }

    fn make_room(&mut self) -> Result<Choice, String> {
        if self.owners.len() < CAPACITY { return Ok(Choice::Open); }
        let oldest = self.owners.iter().enumerate()
            .filter(|(_, owner)| Arc::strong_count(owner) == 1)
            .min_by_key(|(_, owner)| *lock(&owner.last_used)).map(|(index, _)| index);
        match oldest {
            Some(index) => Ok(self.retire(index)),
            None => Err("native account lease capacity exhausted".to_owned()),
        }
    }

    fn retire(&self, index: usize) -> Choice {
        let owner = self.owners[index].clone();
        let _ = owner.native.close_account(&owner.scope);
        Choice::Close(owner)
    }

    fn insert(&mut self, native: NativeAccountClient, scope: AccountScope,
        session: Session, limits: NativeHttpLimits, proxy: Option<String>) -> AccountLease {
        let owner = Arc::new(Retained {
            native, scope, session, limits, proxy, last_used: Mutex::new(Instant::now()),
        });
        self.owners.push(owner.clone());
        AccountLease { owner, reusable: false }
    }
}
