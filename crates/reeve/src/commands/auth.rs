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
    /// List the apps connected to your account through MCP sign-in, or revoke one.
    #[command(args_conflicts_with_subcommands = true)]
    Connections {
        /// List this person's connections instead of your own (admins only).
        #[arg(long)]
        owner: Option<String>,
        #[command(subcommand)]
        action: Option<ConnectionsAction>,
    },
}

#[derive(Subcommand)]
pub enum ConnectionsAction {
    /// Revoke one connection by the id `reeve auth connections` shows: it stops working at once.
    Revoke {
        /// The connection's id.
        id: i64,
    },
}

/// One line per connection: the id (what `revoke` takes), the app, what it may do, and when it ends, ended and
/// why, or was revoked.
fn connection_line(
    c: &reeve_protocol::ConnectionDto,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    use reeve_protocol::{RevokedReason, Scope};
    let app = if c.client_host.is_empty() {
        c.client_name.clone()
    } else {
        format!("{} ({})", c.client_host, c.client_name)
    };
    let scope = match c.scope {
        Scope::Read => "read",
        Scope::Write => "read and change",
    };
    let day = |at: &chrono::DateTime<chrono::Utc>| at.format("%Y-%m-%d").to_string();
    let state = match (&c.revoked_at, c.revoked_reason) {
        // The words of the web page: a person revoked it, or it ended for a reason of the system's.
        (Some(at), Some(reason)) => match reason {
            RevokedReason::Revoked => format!("revoked {}", day(at)),
            RevokedReason::RefreshTokenReused => format!(
                "ended {}: the app's sign-in went out of step; connect it again",
                day(at)
            ),
            RevokedReason::UserDisabled => format!("ended {}: the account was disabled", day(at)),
            RevokedReason::Restored => {
                format!("ended {}: the instance was restored from a backup", day(at))
            }
        },
        _ if c.ends_at <= now => format!("ended {}", day(&c.ends_at)),
        _ => format!("ends {}", day(&c.ends_at)),
    };
    format!("{}\t{app}\t{scope}\t{state}", c.id)
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
        Auth::Connections {
            owner,
            action: None,
        } => {
            let r = crate::config::resolve()?;
            let c = Client::new(r.url, r.token);
            let query: Vec<(&str, String)> = owner.into_iter().map(|o| ("owner", o)).collect();
            let out = c
                .get_query::<Vec<reeve_protocol::ConnectionDto>>("/connections", &query)
                .await?;
            let now = chrono::Utc::now();
            Ok(finish(out, json, |rows| {
                if rows.is_empty() {
                    "no connected apps".to_string()
                } else {
                    rows.iter()
                        .map(|c| connection_line(c, now))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }))
        }
        Auth::Connections {
            action: Some(ConnectionsAction::Revoke { id }),
            ..
        } => {
            let r = crate::config::resolve()?;
            let c = Client::new(r.url, r.token);
            let out = c
                .delete::<reeve_protocol::Empty>(&format!("/connections/{id}"))
                .await?;
            Ok(finish(out, json, |_| format!("revoked connection {id}")))
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
