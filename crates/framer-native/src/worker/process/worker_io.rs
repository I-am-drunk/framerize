use std::io;
use std::time::Instant;
use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use tokio::io::unix::AsyncFd;

pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut pair = [-1; 2];
    if unsafe { libc::pipe2(pair.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(pair[0]), OwnedFd::from_raw_fd(pair[1])) })
}

pub fn child_copy(fd: &OwnedFd) -> io::Result<OwnedFd> {
    let copy = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if copy < 0 { Err(io::Error::last_os_error()) }
    else { Ok(unsafe { OwnedFd::from_raw_fd(copy) }) }
}

pub fn asynchronous(fd: OwnedFd) -> io::Result<AsyncFd<OwnedFd>> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
    } < 0 { return Err(io::Error::last_os_error()); }
    AsyncFd::new(fd)
}

pub async fn read_bounded(fd: AsyncFd<OwnedFd>, maximum: usize) -> io::Result<Vec<u8>> {
    read_with_receipt(fd, maximum).await.map(|(bytes, _)| bytes)
}

pub async fn read_with_receipt(fd: AsyncFd<OwnedFd>, maximum: usize)
    -> io::Result<(Vec<u8>, Option<Instant>)> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut received_at = None;
    loop {
        let mut ready = fd.readable().await?;
        let read = ready.try_io(|fd| {
            let count = unsafe { libc::read(fd.get_ref().as_raw_fd(),
                chunk.as_mut_ptr().cast(), chunk.len()) };
            if count < 0 { Err(io::Error::last_os_error()) } else { Ok(count as usize) }
        });
        let count = match read { Ok(value) => value?, Err(_) => continue };
        if count == 0 { return Ok((bytes, received_at)); }
        // EOF and process exit may arrive later. Neither renews this time.
        received_at = Some(Instant::now());
        if bytes.len().saturating_add(count) > maximum {
            bytes.fill(0); chunk.fill(0);
            return Err(io::Error::new(io::ErrorKind::InvalidData, "pipe limit"));
        }
        bytes.extend_from_slice(&chunk[..count]);
        chunk[..count].fill(0);
    }
}

pub async fn write_all(fd: AsyncFd<OwnedFd>, mut bytes: Vec<u8>) -> io::Result<()> {
    let mut offset = 0;
    let result = async {
        while offset < bytes.len() {
            let mut ready = fd.writable().await?;
            let write = ready.try_io(|fd| {
                let count = unsafe { libc::write(fd.get_ref().as_raw_fd(),
                    bytes[offset..].as_ptr().cast(), bytes.len() - offset) };
                if count < 0 { Err(io::Error::last_os_error()) } else { Ok(count as usize) }
            });
            let count = match write { Ok(value) => value?, Err(_) => continue };
            if count == 0 { return Err(io::Error::from(io::ErrorKind::WriteZero)); }
            offset += count;
        }
        Ok(())
    }.await;
    bytes.fill(0);
    result
}
