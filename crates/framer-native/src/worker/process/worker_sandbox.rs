use std::io;

const fn op(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn filter() -> io::Result<()> {
    let mut rules = [
        op(0x20, 0, 0, 4), op(0x15, 1, 0, 0xc000003e),
        op(0x06, 0, 0, 0x80000000), op(0x20, 0, 0, 0),
        op(0x35, 0, 1, 0x40000000), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_socket as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_socketpair as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_io_uring_setup as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_io_uring_enter as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_io_uring_register as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_fork as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_vfork as u32), op(0x06, 0, 0, 0x00050001),
        op(0x15, 0, 1, libc::SYS_clone3 as u32), op(0x06, 0, 0, 0x00050026),
        op(0x15, 0, 3, libc::SYS_clone as u32), op(0x20, 0, 0, 16),
        op(0x45, 1, 0, libc::CLONE_THREAD as u32),
        op(0x06, 0, 0, 0x00050001), op(0x06, 0, 0, 0x7fff0000),
    ];
    let program = libc::sock_fprog { len: rules.len() as u16, filter: rules.as_mut_ptr() };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::syscall(libc::SYS_seccomp, 1u32, 1u32, &program) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn filter() -> io::Result<()> { Err(io::Error::from(io::ErrorKind::Unsupported)) }

fn verify_socket_denial() -> io::Result<()> {
    for family in [libc::AF_UNIX, libc::AF_INET, libc::AF_INET6] {
        let fd = unsafe { libc::socket(family, libc::SOCK_STREAM, 0) };
        if fd >= 0 { unsafe { libc::close(fd); } return Err(io::ErrorKind::PermissionDenied.into()); }
        if io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut pair = [-1; 2];
        if unsafe { libc::socketpair(family, libc::SOCK_STREAM, 0, pair.as_mut_ptr()) } == 0 {
            unsafe { libc::close(pair[0]); libc::close(pair[1]); }
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}

fn verify_ring_denial() -> io::Result<()> {
    for number in [libc::SYS_io_uring_setup, libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register] {
        let result = unsafe { libc::syscall(number, -1i32, 0usize, 0usize,
            0usize, 0usize, 0usize) };
        if result != -1 || io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}

/// Child-only pre_exec hook: no allocation, environment lookup or locks.
pub unsafe fn prepare(channels: [i32; 3]) -> io::Result<()> {
    for (index, source) in channels.into_iter().enumerate() {
        if libc::dup2(source, index as i32 + 3) < 0 { return Err(io::Error::last_os_error()); }
    }
    // Keep std::process's exec-error pipe usable until exec, then close it
    // together with every unrelated descriptor. Linux CLOSE_RANGE_CLOEXEC.
    if libc::syscall(libc::SYS_close_range, 6u32, u32::MAX, 4u32) != 0 {
        return Err(io::Error::last_os_error());
    }
    let limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
        return Err(io::Error::last_os_error());
    }
    filter()?;
    verify_socket_denial()?;
    verify_ring_denial()
}
