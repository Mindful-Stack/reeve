#[tokio::main]
async fn main() {
    // `process::exit` is load-bearing: returning would drop the tokio runtime, which waits for blocking tasks, and the
    // TUI's browser opener runs on one (it waits for the opener's exit status), so a hung opener would hang the quit.
    std::process::exit(reeve::cli().await);
}
