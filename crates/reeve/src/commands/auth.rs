use super::finish;
use crate::client::{Client, Outcome};
use clap::Subcommand;
use reeve_protocol::{ErrorCode, TokenKind};
use std::io::{BufRead, IsTerminal, Read};

#[derive(Subcommand)]
pub enum Auth {
    /// Sign in through the web client: open its token page, then paste the interactive token it issues.
    Login {
        /// Base url of the Reeve server, e.g. https://reeve.example.com (defaults to REEVE_URL, then the stored url).
        #[arg(long)]
        url: Option<String>,
        /// Print the address instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Store the server url and a token you already have (from the token page, or `reeve-server token-issue`).
    SetToken {
        /// Base url of the Reeve server, e.g. https://reeve.example.com.
        #[arg(long)]
        url: String,
        /// The token to store (visible in the process list; prefer --token-stdin).
        #[arg(
            long,
            conflicts_with = "token_stdin",
            required_unless_present = "token_stdin"
        )]
        token: Option<String>,
        /// Read the token from standard input instead of the command line.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Show where the CLI connects and who the server says you are (never prints the secret).
    Status,
}
pub async fn run(a: Auth, json: bool) -> anyhow::Result<i32> {
    match a {
        Auth::Login { url, no_browser } => login(url, no_browser, json).await,
        Auth::SetToken {
            url,
            token,
            token_stdin,
        } => {
            let token = if token_stdin {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s.trim().to_string()
            } else {
                token.unwrap_or_default()
            };
            anyhow::ensure!(!token.is_empty(), "the token is empty");
            crate::config::save(&crate::config::CliConfig {
                url: Some(url),
                token: Some(token),
            })?;
            let path = crate::config::config_path()?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"outcome":"ok","value":{"path": path}})
                );
            } else {
                println!("saved to {}", path.display());
            }
            Ok(0)
        }
        Auth::Status => {
            let r = crate::config::resolve()?;
            let via_env = crate::config::env_nonempty("REEVE_TOKEN").is_some();
            let url = r.url.clone();
            let c = Client::new(r.url, r.token);
            let out = match c.get::<reeve_protocol::Me>("/me").await? {
                Outcome::Ok(me) => Outcome::Ok(serde_json::json!({
                    "url": url, "token_from_env": via_env,
                    "email": me.email, "role": me.role, "token_kind": me.token_kind,
                })),
                Outcome::Refused {
                    reasons,
                    overridable,
                } => Outcome::Refused {
                    reasons,
                    overridable,
                },
                Outcome::Error {
                    code,
                    message,
                    envelope,
                    retry_after,
                    current,
                } => Outcome::Error {
                    code,
                    message,
                    envelope,
                    retry_after,
                    current,
                },
            };
            Ok(finish(out, json, |v| {
                format!(
                    "{} (token from {})\n{} role={} token_kind={}",
                    v["url"].as_str().unwrap_or_default(),
                    if via_env { "REEVE_TOKEN" } else { "config" },
                    v["email"].as_str().unwrap_or_default(),
                    v["role"].as_str().unwrap_or_default(),
                    v["token_kind"].as_str().unwrap_or_default()
                )
            }))
        }
    }
}

/// Report a failed login (nothing has been written) and return exit status 1.
fn refuse(json: bool, code: ErrorCode, message: &str) -> i32 {
    if json {
        println!(
            "{}",
            serde_json::json!({"outcome":"error","code":code,"message":message})
        );
    }
    eprintln!("error: {message}");
    1
}

/// `reeve auth login`: the token page issues an interactive token, the user pastes it here, and it is stored only after
/// `GET /me` with that very token says it is a live interactive token (kb/ADR-0015 R7). REEVE_TOKEN plays no part.
async fn login(url: Option<String>, no_browser: bool, json: bool) -> anyhow::Result<i32> {
    let url = url
        .or_else(|| crate::config::env_nonempty("REEVE_URL"))
        .or(crate::config::load().url)
        .ok_or_else(|| anyhow::anyhow!("no server url: pass --url https://reeve.example.com"))?;
    let url = url.trim_end_matches('/').to_string();
    if crate::config::env_nonempty("REEVE_TOKEN").is_some() {
        // REEVE_TOKEN wins over the stored token (kb/ADR-0015 R7), so the login would otherwise look like it did nothing.
        eprintln!(
            "note: REEVE_TOKEN is set and takes precedence over the token stored here; unset it to use this login"
        );
    }
    let page = format!("{url}/tokens?for=cli");
    eprintln!("Open {page}, sign in with Google, create an interactive token and paste it here:");
    // Only a person at a terminal gets a browser; piped input (tests, CI, scripts) never does.
    let stdin = std::io::stdin();
    if stdin.is_terminal() && !no_browser {
        open_browser(&page);
    }
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let token = line.trim().to_string();
    if !token.starts_with("reeve_") {
        return Ok(refuse(
            json,
            ErrorCode::Validation,
            "that is not a Reeve token (tokens start with reeve_)",
        ));
    }
    let me = match Client::new(url.clone(), token.clone())
        .get::<reeve_protocol::Me>("/me")
        .await?
    {
        Outcome::Ok(me) => me,
        Outcome::Error { code, message, .. } => {
            let message = if code == ErrorCode::Unauthenticated {
                "the server did not accept that token (expired, revoked or mistyped)".to_string()
            } else {
                message
            };
            return Ok(refuse(json, code, &message));
        }
        Outcome::Refused { .. } => {
            return Ok(refuse(
                json,
                ErrorCode::Internal,
                "unexpected response from /me",
            ));
        }
    };
    if me.token_kind != TokenKind::Interactive {
        return Ok(refuse(
            json,
            ErrorCode::Validation,
            "automation tokens go in REEVE_TOKEN; login stores an interactive token",
        ));
    }
    crate::config::save(&crate::config::CliConfig {
        url: Some(url),
        token: Some(token),
    })?;
    if json {
        println!(
            "{}",
            serde_json::json!({"outcome":"ok","value":{"email": me.email, "kind": "interactive"}})
        );
    } else {
        println!(
            "signed in as {} (saved to {})",
            me.email,
            crate::config::config_path()?.display()
        );
    }
    Ok(0)
}

/// Best effort: the address is already on stderr, so a missing opener is not an error.
fn open_browser(page: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(page)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
