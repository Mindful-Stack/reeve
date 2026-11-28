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
        crate::terminal::line(&c.client_name).into_owned()
    } else {
        format!(
            "{} ({})",
            crate::terminal::line(&c.client_host),
            crate::terminal::line(&c.client_name)
        )
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

/// `reeve auth status` in text mode: where the CLI connects, who the server says you are, and both builds. Each value
/// is the configuration's or the server's text and prints as one line (`terminal::line`).
fn status_text(v: &serde_json::Value, via_env: bool) -> String {
    use crate::terminal::line;
    let field = |name: &str| line(v[name].as_str().unwrap_or_default()).into_owned();
    let build = |x: &serde_json::Value, none: &str| {
        x.as_str().map_or_else(
            || none.to_string(),
            |s| format!("build {}", line(crate::cli_version::short(s))),
        )
    };
    format!(
        "{} (token from {})\n{} role={} token_kind={}\ncli: {} (server offers: {})",
        field("url"),
        if via_env { "REEVE_TOKEN" } else { "config" },
        field("email"),
        field("role"),
        field("token_kind"),
        build(&v["cli"]["current"], "dev build"),
        build(&v["cli"]["server"], "none"),
    )
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
            let written = if json {
                crate::terminal::exact(
                    &serde_json::json!({"outcome":"ok","value":{"path": path}}).to_string(),
                )
            } else {
                crate::terminal::out(&format!("saved to {}", path.display()));
                Ok(())
            };
            Ok(crate::terminal::status(written, 0))
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
                    "cli": {"current": crate::cli_version::current(), "server": crate::cli_version::offered()},
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
            Ok(finish(out, json, |v| status_text(v, via_env)))
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
    let written = if json {
        crate::terminal::exact(
            &serde_json::json!({"outcome":"error","code":code,"message":message}).to_string(),
        )
    } else {
        Ok(())
    };
    crate::terminal::err(&format!("error: {message}"));
    crate::terminal::status(written, 1)
}

/// `reeve auth login`: the token page issues an interactive token, the user pastes it here, and it is stored only after
/// `GET /me` with that very token says it is a live interactive token (kb/ADR-0015 R7). REEVE_TOKEN plays no part.
async fn login(url: Option<String>, no_browser: bool, json: bool) -> anyhow::Result<i32> {
    let url = url
        .or_else(|| crate::config::env_nonempty("REEVE_URL"))
        .or(crate::config::load().url)
        .ok_or_else(|| anyhow::anyhow!(crate::config::NO_SERVER_URL))?;
    let url = url.trim_end_matches('/').to_string();
    if crate::config::env_nonempty("REEVE_TOKEN").is_some() {
        // REEVE_TOKEN wins over the stored token (kb/ADR-0015 R7), so the login would otherwise look like it did nothing.
        crate::terminal::err(
            "note: REEVE_TOKEN is set and takes precedence over the token stored here; unset it to use this login",
        );
    }
    let page = format!("{url}/tokens?for=cli");
    crate::terminal::err(&format!(
        "Open {page}, sign in with Google, create an interactive token and paste it here:"
    ));
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
    let written = if json {
        crate::terminal::exact(
            &serde_json::json!({"outcome":"ok","value":{"email": me.email, "kind": "interactive"}})
                .to_string(),
        )
    } else {
        crate::terminal::out(&signed_in(&me, &crate::config::config_path()?));
        Ok(())
    };
    Ok(crate::terminal::status(written, 0))
}

/// `reeve auth login`'s last line. The account is the server's text, so it prints as one line (`terminal::line`).
fn signed_in(me: &reeve_protocol::Me, path: &std::path::Path) -> String {
    format!(
        "signed in as {} (saved to {})",
        crate::terminal::line(&me.email),
        path.display()
    )
}

/// Best effort: the address is already on stderr, so a missing opener is not an error.
fn open_browser(page: &str) {
    if let Ok(mut c) = crate::browser::command(page) {
        let _ = c.spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostile::{Paths, assert_one_line, assert_shown};
    use reeve_protocol::{ConnectionDto, Me, Role, Scope};

    fn connection() -> ConnectionDto {
        ConnectionDto {
            id: 3,
            owner: "dan@example.com".into(),
            client_id: "https://claude.ai/oauth/mcp-client".into(),
            client_host: "claude.ai".into(),
            client_name: "Claude".into(),
            scope: Scope::Write,
            approved_at: "2026-11-01T10:00:00Z".parse().unwrap(),
            ends_at: "2026-12-01T10:00:00Z".parse().unwrap(),
            refreshed_at: Some("2026-11-15T10:00:00Z".parse().unwrap()),
            revoked_at: None,
            revoked_reason: None,
        }
    }

    /// What `connection` leaves empty: it is live.
    fn connection_paths() -> Paths {
        Paths::none()
            .absent("/revoked_at", "a live connection")
            .absent("/revoked_reason", "a live connection")
    }

    /// A connected app names itself: its host and name are another party's text, and stay in their column.
    #[test]
    fn a_connections_app_stays_in_its_column() {
        let now: chrono::DateTime<chrono::Utc> = "2026-11-20T10:00:00Z".parse().unwrap();
        assert_shown(
            &assert_one_line(&connection(), &connection_paths(), |c| {
                connection_line(c, now)
            }),
            &["/client_host", "/client_name"],
        );
        // Without a host the name stands alone (the helper cannot reach this arm: it makes the empty host hostile).
        let bare = ConnectionDto {
            client_host: String::new(),
            client_name: "a\nb\tc".into(),
            ..connection()
        };
        assert_eq!(
            connection_line(&bare, now),
            "3\ta\\nb\\tc\tread and change\tends 2026-12-01"
        );
    }

    /// `reeve auth status`: the url, the account, its role and token kind, and both builds, each on its line.
    #[test]
    fn auth_status_prints_each_value_in_its_place() {
        let v = serde_json::json!({
            "url": "https://reeve.example.com", "token_from_env": false,
            "email": "dan@example.com", "role": "admin", "token_kind": "interactive",
            "cli": {"current": "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432",
                    "server": "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b"},
        });
        assert_shown(
            &assert_one_line(&v, &Paths::none(), |v| status_text(v, false)),
            &[
                "/url",
                "/email",
                "/role",
                "/token_kind",
                "/cli/current",
                "/cli/server",
            ],
        );
    }

    /// `reeve auth login` names the account the server answered for, on one line.
    #[test]
    fn login_names_the_account_on_one_line() {
        let me = Me {
            email: "dan@example.com".into(),
            role: Role::Member,
            token_kind: TokenKind::Interactive,
        };
        assert_eq!(
            assert_one_line(&me, &Paths::none(), |m| {
                signed_in(m, std::path::Path::new("reeve/config.toml"))
            }),
            ["/email"]
        );
    }
}
