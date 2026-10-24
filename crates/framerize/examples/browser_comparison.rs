//! Runs only through an explicit inherited private controller socket.
#[path = "../src/observed_authorization.rs"]
mod observed_authorization;
#[path = "../src/one_shot_control.rs"]
mod one_shot_control;
#[path = "../src/browser_comparison_child.rs"]
mod child;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 || args[1] != "--control-fd" {
        return std::process::ExitCode::from(2);
    }
    let Ok(fd) = args[2].parse::<i32>() else { return std::process::ExitCode::from(2); };
    std::process::ExitCode::from(match child::execute(fd).await {
        Ok(true) => 0, Ok(false) => 1, Err(()) => 2,
    })
}
