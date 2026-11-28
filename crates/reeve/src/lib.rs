//! The `reeve` CLI. `main.rs` is the binary; the library target exists so the TUI's integration tests can drive
//! `tui::Driver`. Every module except `tui` stays private to the crate.
mod atomic_file;
mod browser;
mod cli_version;
mod client;
mod commands;
mod config;
mod git;
pub mod glyphs;
#[cfg(test)]
mod hostile;
mod jira;
mod promise;
mod relations;
mod render;
mod terminal;
pub mod tui;
mod urgency;

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
    /// `current`, the version to re-read from (-1 for a concurrent update with no version to give: re-read and retry).
    #[arg(long, global = true)]
    json: bool,
    /// Mark keys with plain text ([I], [E], [S], [B], [C]) instead of glyphs, for a terminal that cannot show them; also
    /// on when REEVE_ASCII is set.
    #[arg(long, global = true)]
    ascii: bool,
    #[command(subcommand)]
    cmd: commands::Cmd,
}

/// Parses the arguments, runs the command and returns the exit status. After the command's own output, whatever its
/// outcome, prints the update notice (kb/ADR-0030 R6) -- the one call of `cli_version::notify`, so at most once per
/// process -- except for `reeve tui`, which shows a header note instead.
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
    let tui = matches!(cli.cmd, commands::Cmd::Tui);
    let code = match commands::run(cli.cmd, cli.json).await {
        Ok(code) => code,
        Err(e) => {
            let written = if cli.json {
                terminal::exact(
                    &serde_json::json!({"outcome":"error","code":"internal","message":format!("{e:#}")})
                        .to_string(),
                )
            } else {
                Ok(())
            };
            terminal::err(&format!("error: {e:#}"));
            terminal::status(written, 1)
        }
    };
    if !tui {
        cli_version::notify();
    }
    code
}
