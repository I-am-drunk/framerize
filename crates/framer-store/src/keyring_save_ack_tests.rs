use super::{save_keyring_entry, FramerUserId, Session};
use keyring::{credential::CredentialApi, Entry, Error};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Record {
    secret: Option<Vec<u8>>,
    failure: Option<Error>,
    writes: usize,
}

struct RecordingCredential(Arc<Mutex<Record>>);

fn session(cookie: &str) -> Session {
    Session {
        active_user: FramerUserId("fixture-user".into()),
        cookie_header: cookie.into(),
    }
}

impl CredentialApi for RecordingCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        let mut record = self.0.lock().unwrap();
        record.writes += 1;
        if let Some(error) = record.failure.take() { return Err(error); }
        record.secret = Some(secret.to_vec());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.0.lock().unwrap().secret.clone().ok_or(Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.0.lock().unwrap().secret.take().map(|_| ()).ok_or(Error::NoEntry)
    }
    fn as_any(&self) -> &dyn std::any::Any { self }
}

fn recording_entry(failure: Option<Error>) -> (Entry, Arc<Mutex<Record>>) {
    let record = Arc::new(Mutex::new(Record {
        secret: Some(b"session=previous".to_vec()), failure, writes: 0,
    }));
    let entry = Entry::new_with_credential(Box::new(RecordingCredential(record.clone())));
    (entry, record)
}

#[test]
fn mock_entry_refuses_save_without_replacement() {
    for previous in [None, Some("session=previous")] {
        for next in ["session=next", ""] {
            let entry = Entry::new_with_credential(
                Box::new(keyring::mock::MockCredential::default()),
            );
            if let Some(value) = previous { entry.set_password(value).unwrap(); }
            let result = save_keyring_entry(&entry, &session(next));
            assert_eq!(result, Err("session keyring backend is not persistent".into()));
            match previous {
                Some(value) => assert_eq!(entry.get_password().unwrap(), value),
                None => assert!(matches!(entry.get_password(), Err(Error::NoEntry))),
            }
        }
    }
}

#[test]
fn nonmock_entry_preserves_full_cookie_bytes() {
    for cookie in ["", " session=fixture=opaque; 終\0 "] {
        let (entry, record) = recording_entry(None);
        assert_eq!(save_keyring_entry(&entry, &session(cookie)), Ok(()));
        let record = record.lock().unwrap();
        assert_eq!(record.secret.as_deref(), Some(cookie.as_bytes()));
        assert_eq!(record.writes, 1);
    }
}

#[test]
fn nonmock_entry_preserves_write_error_and_old_value() {
    for error in [Error::NoEntry, Error::PlatformFailure(Box::new(
        std::io::Error::new(std::io::ErrorKind::Other, "fixture write failed"),
    ))] {
        let expected = error.to_string();
        let (entry, record) = recording_entry(Some(error));
        assert_eq!(save_keyring_entry(&entry, &session("session=next")), Err(expected));
        let record = record.lock().unwrap();
        assert_eq!(record.secret.as_deref(), Some(b"session=previous".as_slice()));
        assert_eq!(record.writes, 1);
    }
}
