//! Bounded versions retained only by live jobs and failed mutation barriers.
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Key { User(FramerUserId), Account(AccountId) }
pub(super) struct Stamp { identity: Arc<()>, epoch: u64 }
struct Version { key: Key, identity: Arc<()>, epoch: u64, blocked: bool }
#[derive(Default)]
pub(super) struct Versions { entries: Vec<Version> }

impl Versions {
    pub(super) fn prune(&mut self) {
        self.entries.retain(|v| v.blocked || Arc::strong_count(&v.identity) > 1);
    }
    fn entry(&mut self, key: Key) -> Result<&mut Version, String> {
        self.prune();
        let index = match self.entries.iter().position(|v| v.key == key) {
            Some(index) => index,
            None => {
                // Two versions per live job plus ten failed barriers per kind.
                if self.entries.len() >= 4 * super::CAPACITY {
                    return Err("native session version capacity exhausted".to_owned());
                }
                self.entries.push(Version { key, identity: Arc::new(()), epoch: 0, blocked: false });
                self.entries.len() - 1
            }
        };
        Ok(&mut self.entries[index])
    }
    pub(super) fn read(&mut self, key: Key) -> Result<Stamp, String> {
        let version = self.entry(key)?;
        if version.blocked { return Err(super::CHANGED.to_owned()); }
        Ok(Stamp { identity: version.identity.clone(), epoch: version.epoch })
    }
    pub(super) fn change<T>(&mut self, key: Key, begin: impl FnOnce() -> Result<T, String>)
        -> Result<(Stamp, T), String> {
        let version = self.entry(key)?;
        let epoch = version.epoch.checked_add(1).ok_or(super::STOPPED)?;
        // A failed native claim must not change this version or its fence.
        let ticket = begin()?;
        version.epoch = epoch;
        version.blocked = true;
        Ok((Stamp { identity: version.identity.clone(), epoch }, ticket))
    }
    pub(super) fn current(&self, stamp: &Stamp) -> bool {
        self.entries.iter().any(|v| Arc::ptr_eq(&v.identity, &stamp.identity)
            && v.epoch == stamp.epoch && !v.blocked)
    }
    pub(super) fn unblock(&mut self, stamp: &Stamp) {
        let version = self.entries.iter_mut().find(|v| Arc::ptr_eq(&v.identity, &stamp.identity))
            .expect("held stamps retain their version");
        assert_eq!(version.epoch, stamp.epoch, "exclusive mutation owns this version");
        version.blocked = false;
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize { self.entries.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn user(name: &str) -> Key { Key::User(FramerUserId(name.to_owned())) }

    #[test]
    fn user_changes_invalidate_only_their_own_retained_versions() {
        let mut versions = Versions::default();
        let a = versions.read(user("a")).unwrap();
        let b = versions.read(user("b")).unwrap();
        let (change, ()) = versions.change(user("a"), || Ok(())).unwrap();
        versions.unblock(&change);
        assert!(!versions.current(&a));
        assert!(versions.current(&b));
        drop((a, b, change));
        versions.prune();
        assert_eq!(versions.len(), 0);
    }

    #[test]
    fn historical_users_are_pruned_while_failed_barriers_stay_bounded() {
        let mut versions = Versions::default();
        for index in 0..1000 {
            drop(versions.read(user(&format!("history-{index}"))).unwrap());
            versions.prune();
            assert_eq!(versions.len(), 0);
        }
        for index in 0..(4 * super::super::CAPACITY) {
            drop(versions.change(user(&format!("failed-{index}")), || Ok(())).unwrap());
        }
        assert!(versions.read(user("overflow")).is_err());
        let (retry, ()) = versions.change(user("failed-0"), || Ok(())).unwrap();
        versions.unblock(&retry);
        drop(retry);
        versions.prune();
        assert!(versions.read(user("new" )).is_ok());
    }
}
