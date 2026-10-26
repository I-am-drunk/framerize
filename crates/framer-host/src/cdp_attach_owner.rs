use framer_core::AccountId;
use std::sync::Mutex;

/// Attachment does not create an isolated browser context for each account.
/// Keep its account claim through failed/cancelled starts and disconnection.
#[derive(Default)]
pub(super) struct AttachOwner(Mutex<Option<AccountId>>);

impl AttachOwner {
    pub(super) fn claim(&self, account: AccountId) -> Result<(), String> {
        let mut owner = self.0.lock()
            .map_err(|_| "attached browser ownership is unavailable".to_owned())?;
        if owner.is_some_and(|existing| existing != account) {
            return Err("attached browser is already assigned to another account".into());
        }
        *owner = Some(account);
        Ok(())
    }
}
