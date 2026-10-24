//! Private inherited socket. No listener, credential file or stdout protocol.
use serde::{de::DeserializeOwned, Serialize};
use std::{os::fd::{FromRawFd, OwnedFd}, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::UnixStream};

pub(super) struct Channel(pub(super) UnixStream);
impl Channel {
    pub(super) fn inherited(fd: i32) -> Result<Self, ()> {
        if !(3..=1024).contains(&fd) { return Err(()); }
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 { return Err(()); }
        // The explicit descriptor is transferred once and never inherited by a host.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 { return Err(()); }
        let socket = std::os::unix::net::UnixStream::from(owned);
        socket.set_nonblocking(true).map_err(|_| ())?;
        let stream = UnixStream::from_std(socket).map_err(|_| ())?;
        let peer = stream.peer_cred().map_err(|_| ())?;
        if peer.uid() != unsafe { libc::geteuid() }
            || peer.pid() != Some(unsafe { libc::getppid() }) { return Err(()); }
        Ok(Self(stream))
    }
    pub(super) async fn receive<T: DeserializeOwned>(&mut self) -> Result<T, ()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let size = self.0.read_u32().await.map_err(|_| ())? as usize;
            if size == 0 || size > 96 * 1024 { return Err(()); }
            let mut bytes = vec![0; size];
            self.0.read_exact(&mut bytes).await.map_err(|_| ())?;
            serde_json::from_slice(&bytes).map_err(|_| ())
        }).await.map_err(|_| ())?
    }
    pub(super) async fn send(&mut self, value: &impl Serialize) -> Result<(), ()> {
        let bytes = serde_json::to_vec(value).map_err(|_| ())?;
        if bytes.is_empty() || bytes.len() > 8 * 1024 { return Err(()); }
        tokio::time::timeout(Duration::from_secs(5), async {
            self.0.write_u32(bytes.len() as u32).await.map_err(|_| ())?;
            self.0.write_all(&bytes).await.map_err(|_| ())
        }).await.map_err(|_| ())?
    }
}
