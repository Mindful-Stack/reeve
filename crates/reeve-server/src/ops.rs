//! Operator subcommands that need neither the configuration nor the database: the container healthcheck and the
//! migration listing a deploy can compare with the database's ledger before it migrates.
use anyhow::{Context, bail};
use std::time::Duration;

/// The healthcheck's whole-request timeout: above `/health`'s own database timeout (`HEALTH_DB_TIMEOUT`, 3 s), so a
/// slow database is answered 503 rather than timed out, and below compose.yml's healthcheck `timeout` (10 s).
pub const HEALTHCHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// `reeve-server healthcheck`: GET `<base>/health`; Ok only on 200. No proxy: the target is always the server's own
/// loopback port.
pub async fn healthcheck(base: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(base).with_context(|| format!("--url {base:?} is not a URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("--url must be http or https, got {base:?}");
    }
    let url = format!("{}/health", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(HEALTHCHECK_TIMEOUT)
        .build()
        .context("building the HTTP client")?;
    let res = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if res.status() != reqwest::StatusCode::OK {
        bail!("GET {url} answered {}", res.status());
    }
    Ok(())
}

/// `reeve-server migrations`: one `<version> <checksum hex>` line per migration of the composed migrator `serve` runs,
/// in version order. The checksum is lower-case hex, as `encode(checksum, 'hex')` prints `_sqlx_migrations.checksum`.
pub fn migration_listing() -> String {
    crate::state::composed_migrator()
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| format!("{} {}\n", m.version, hex::encode(&*m.checksum)))
        .collect()
}
