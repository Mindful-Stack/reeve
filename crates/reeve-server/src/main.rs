use anyhow::Context;
use clap::{Parser, Subcommand};
use reeve_server::*;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "reeve-server", version)]
struct Cli {
    #[arg(long, env = "REEVE_CONFIG", default_value = "reeve.toml")]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run migrations and serve the API.
    Serve,
    /// Run migrations and exit.
    Migrate,
    /// Operator command, run on the host with database access: issue a token for an allowlisted email and print the secret once.
    /// This is not a sign-in path (kb/ADR-0015 R1); users sign in with Google in plan B, which issues interactive tokens.
    TokenIssue {
        #[arg(long)]
        email: String,
        #[arg(long, value_enum, default_value = "interactive")]
        kind: KindArg,
        #[arg(long, default_value = "")]
        label: String,
    },
    /// Developer command, only in builds with the `static-oidc` feature (the Dockerfile never enables it): issue an
    /// interactive token whose audience is the MCP endpoint for an allowlisted email, valid for one hour
    /// (kb/ADR-0029 R5), and print the secret once. It makes a connection of its own (client
    /// `urn:reeve:developer-command`, scope write), which disabling its owner revokes.
    #[cfg(feature = "static-oidc")]
    McpTokenIssue {
        #[arg(long)]
        email: String,
    },
    /// Operator command, run on the host with database access: write the whole instance as JSON (the document
    /// `GET /api/v1/export` returns) without a token. Read-only; never migrates; refuses a database whose migrations
    /// differ from this binary's.
    Export {
        /// Write to this file (mode 0600, replaced atomically) instead of standard output.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Read this database on the configured server instead of the one `database_url` names (a rehearsal's
        /// scratch database); host, user and password still come from the configuration.
        #[arg(long)]
        database: Option<String>,
        /// Read inside this exported snapshot (`pg_export_snapshot()` in a REPEATABLE READ transaction the caller
        /// keeps open), so the document matches a `pg_dump --snapshot` of the same id (scripts/backup.sh).
        #[arg(long)]
        snapshot: Option<String>,
        /// The document's `exported_at` (RFC 3339), and the instant urgency is computed at (an urgent mark is active
        /// until one cycle after it was set). Default: now. A restore rehearsal passes the live export's `exported_at`,
        /// so a mark expiring between the backup and the rehearsal does not differ.
        #[arg(long)]
        at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Disable a user and revoke their tokens.
    UserDisable {
        #[arg(long)]
        email: String,
    },
    /// Operator command: print who may sign in and who is admin, merged from the configuration and the database, one
    /// `kind<TAB>value<TAB>source<TAB>flag` line per entry (flag is `not allowed` or empty; control characters in
    /// value and source are escaped). Read-only: connects read-only, never migrates and refuses a database at another
    /// migration. The floor in reeve.toml is the break-glass path, so there is no add or remove here.
    AccessList,
    /// Print `name<TAB>url` for every `[[repos]]` entry that has a url: what the scheduled decision check clones. Reads
    /// the configuration only; never connects to the database.
    Repos,
    /// Container healthcheck and deploy gate: GET `<url>/health` with a five-second timeout; exit 0 on 200, 1
    /// otherwise. Needs neither the configuration nor the database (the runtime image has no curl).
    Healthcheck {
        #[arg(long, default_value = "http://127.0.0.1:7474")]
        url: String,
    },
    /// Print the binary's embedded migrations, one `<version> <checksum hex>` per line, from the migrator `serve` runs.
    /// Needs neither the configuration nor the database; a deploy can compare it with `_sqlx_migrations`.
    Migrations,
    /// Load and validate the configuration exactly as `serve` does first, touch nothing, print `configuration ok`.
    CheckConfig,
}
#[derive(Clone, Copy, clap::ValueEnum)]
enum KindArg {
    Interactive,
    Automation,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        // No colour codes in a log file (the smoke's server.log, a systemd journal).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    // Neither needs the configuration: the healthcheck runs every few seconds in the container, `migrations` before
    // any database exists.
    if let Cmd::Healthcheck { url } = &cli.cmd {
        return healthcheck(url).await;
    }
    if let Cmd::Migrations = cli.cmd {
        print!("{}", migration_listing());
        return Ok(());
    }
    let config = ServerConfig::load(&cli.config)?;
    // `load` parses and validates; that is everything `serve` checks before it connects.
    if let Cmd::CheckConfig = cli.cmd {
        println!("configuration ok");
        return Ok(());
    }
    if let Cmd::Export {
        out,
        database,
        snapshot,
        at,
    } = &cli.cmd
    {
        // Validated before connecting: the id is spliced into SET TRANSACTION SNAPSHOT.
        let snapshot = snapshot.as_deref().map(SnapshotId::parse).transpose()?;
        // One timestamp, taken before the snapshot's first statement (kb/ADR-0010 R4), unless the caller names it.
        let text = offline_export(
            &config,
            database.as_deref(),
            snapshot.as_ref(),
            at.unwrap_or_else(chrono::Utc::now),
        )
        .await?;
        match out {
            Some(path) => {
                write_atomically(path, text.as_bytes())
                    .with_context(|| format!("writing {}", path.display()))?;
                eprintln!("exported to {}", path.display());
            }
            None => {
                use std::io::Write;
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(text.as_bytes())?;
                stdout.flush()?;
            }
        }
        return Ok(());
    }
    if let Cmd::AccessList = cli.cmd {
        let pool = connect_offline(&config.database_url, None).await?;
        check_migrations(&pool).await?;
        let policy = reeve_identity::make_access_store(pool.clone())
            .load(&config.identity())
            .await
            .context("loading the access lists")?;
        let v = policy.view();
        let mut text = String::new();
        for (kind, rows) in [
            ("domain", &v.domains),
            ("email", &v.emails),
            ("admin", &v.admins),
        ] {
            for r in rows {
                let source = match &r.source {
                    reeve_identity::AccessSource::Config => "reeve.toml".to_string(),
                    reeve_identity::AccessSource::Database { added_by, added_at } => {
                        format!("added by {added_by} {}", added_at.format("%Y-%m-%d"))
                    }
                };
                let flag = if r.allowed { "" } else { "not allowed" };
                text.push_str(&format!(
                    "{kind}\t{}\t{}\t{flag}\n",
                    escape_control(&r.value),
                    escape_control(&source)
                ));
            }
        }
        pool.close().await;
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(text.as_bytes())?;
        stdout.flush()?;
        return Ok(());
    }
    // Configuration only: the scheduled check calls this inside the server container, and it must answer whether or
    // not the database is up.
    if let Cmd::Repos = cli.cmd {
        let (listing, warnings) = config.repo_listing()?;
        for w in warnings {
            eprintln!("warning: {w}");
        }
        print!("{listing}");
        return Ok(());
    }
    let pool = pool_options().connect(&config.database_url).await?;
    run_migrations(&pool).await?;
    match cli.cmd {
        Cmd::Export { .. } => unreachable!("dispatched above, before the pool and the migrations"),
        Cmd::Repos => unreachable!("handled before connecting"),
        Cmd::Migrations | Cmd::CheckConfig => unreachable!("handled before connecting"),
        Cmd::Healthcheck { .. } => unreachable!("handled before the configuration is loaded"),
        Cmd::Migrate => {}
        Cmd::Serve => {
            let bind = config.bind.clone();
            let state = build_state(config, pool).await?;
            tracing::info!("{}", mcp_startup_line(&state));
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!(%bind, "reeve-server listening");
            axum::serve(listener, app(state)).await?;
        }
        Cmd::TokenIssue { email, kind, label } => {
            let store = reeve_identity::make_token_store(pool.clone());
            let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> =
                reeve_activity::make_activity_store(pool.clone());
            let kind = match kind {
                KindArg::Interactive => reeve_protocol::TokenKind::Interactive,
                KindArg::Automation => reeve_protocol::TokenKind::Automation,
            };
            let policy = reeve_identity::make_access_store(pool.clone())
                .load(&config.identity())
                .await?;
            let t = reeve_identity::issue_token(
                &pool,
                &*store,
                &*log,
                &policy,
                "operator",
                reeve_protocol::Via::System,
                &email,
                kind,
                &label,
                None,
                chrono::Utc::now(),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", t.secret);
        }
        #[cfg(feature = "static-oidc")]
        Cmd::McpTokenIssue { email } => {
            let store = reeve_identity::make_token_store(pool.clone());
            let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> =
                reeve_activity::make_activity_store(pool.clone());
            let policy = reeve_identity::make_access_store(pool.clone())
                .load(&config.identity())
                .await?;
            let now = chrono::Utc::now();
            let t = reeve_identity::issue_mcp_token(
                &pool,
                &*store,
                &*log,
                &policy,
                "operator",
                reeve_protocol::Via::System,
                &email,
                "mcp developer token",
                now + chrono::Duration::hours(1),
                now,
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", t.secret);
        }
        Cmd::UserDisable { email } => {
            let store = reeve_identity::make_token_store(pool.clone());
            let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> =
                reeve_activity::make_activity_store(pool.clone());
            reeve_identity::disable_user(
                &pool,
                &*store,
                &*log,
                "operator",
                reeve_protocol::Via::System,
                &email,
                chrono::Utc::now(),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        }
        Cmd::AccessList => unreachable!("dispatched above, before the migrations"),
    }
    Ok(())
}

/// Escapes only control characters (a tab or newline in a hand-edited row must not break a line); every other
/// character, quotes included, is left as is.
fn escape_control(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
