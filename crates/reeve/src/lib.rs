//! The `reeve` CLI. `main.rs` is the binary; the library target exists so the TUI's integration tests can drive
//! `tui::Driver`. Every module except `tui` stays private to the crate.
mod client;
mod commands;
mod config;
mod git;
pub mod glyphs;
mod jira;
mod render;
pub mod tui;

use clap::Parser;

/// Exit status: 0 success, 2 the server refused (the request with reasons, or the caller: forbidden, unauthenticated),
/// 1 any other failure.
#[derive(Parser)]
#[command(
    name = "reeve-planner",
    bin_name = "reeve",
    version,
    about = "Reeve planner CLI"
)]
struct Cli {
    /// Print machine-readable JSON (the full response envelope) instead of text. A version conflict's envelope carries
    /// `current`, the version to re-read from (-1 when the server gave up on a concurrent update).
    #[arg(long, global = true)]
    json: bool,
    /// Mark keys with plain text ([I], [E], [S], [B], [C]) instead of glyphs, for a terminal that cannot show them; also
    /// on when REEVE_ASCII is set.
    #[arg(long, global = true)]
    ascii: bool,
    #[command(subcommand)]
    cmd: commands::Cmd,
}

/// Parses the arguments, runs the command and returns the exit status.
pub async fn cli() -> i32 {
    // Exit 2 means "refused" only, so usage errors exit 1 (clap would use 2).
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { 1 } else { 0 };
            let _ = e.print();
            return code;
        }
    };
    glyphs::set_ascii(cli.ascii || std::env::var_os("REEVE_ASCII").is_some());
    match commands::run(cli.cmd, cli.json).await {
        Ok(code) => code,
        Err(e) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"outcome":"error","code":"internal","message":format!("{e:#}")})
                );
            }
            eprintln!("error: {e:#}");
            1
        }
    }
}
