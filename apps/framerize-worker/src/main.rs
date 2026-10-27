#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use std::os::fd::{FromRawFd, OwnedFd};
    use tokio::net::unix::pipe::{Receiver, Sender};
    // The launch contract gives this process exclusive ownership of pipes 0/1.
    // Tokio's pipe I/O is cancellable; blocking stdin would prevent joined exit.
    let input = Receiver::from_owned_fd(unsafe { OwnedFd::from_raw_fd(0) });
    let output = Sender::from_owned_fd(unsafe { OwnedFd::from_raw_fd(1) });
    match (input, output) {
        (Ok(input), Ok(output)) => {
            if framerize_worker::runner::run(input, output).await.is_ok() {
                std::process::ExitCode::SUCCESS
            } else { std::process::ExitCode::FAILURE }
        }
        _ => std::process::ExitCode::FAILURE,
    }
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode { std::process::ExitCode::FAILURE }
