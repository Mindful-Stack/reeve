use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CliConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

/// An environment variable counts as unset when it is empty.
pub fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    let dir = if let Some(d) = env_nonempty("REEVE_CONFIG_DIR") {
        PathBuf::from(d)
    } else {
        let base = match (env_nonempty("XDG_CONFIG_HOME"), env_nonempty("HOME")) {
            (Some(x), _) => PathBuf::from(x),
            (None, Some(h)) => PathBuf::from(h).join(".config"),
            (None, None) => anyhow::bail!(
                "cannot locate a config directory: set REEVE_CONFIG_DIR, XDG_CONFIG_HOME or HOME"
            ),
        };
        base.join("reeve")
    };
    Ok(dir.join("config.toml"))
}

pub fn load() -> CliConfig {
    config_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// The file holds a token, so it is written 0600 inside a 0700 directory.
pub fn save(c: &CliConfig) -> anyhow::Result<()> {
    use std::io::Write;
    let p = config_path()?;
    let dir = p.parent().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&p)?;
        // `mode` only applies to a newly created file.
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(toml::to_string(c)?.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
        std::fs::File::create(&p)?.write_all(toml::to_string(c)?.as_bytes())?;
    }
    Ok(())
}

/// Resolution order (kb/ADR-0008 R3): REEVE_TOKEN in the environment is an automation token and wins;
/// otherwise the interactive token stored by `reeve auth set-token` / plan B's `reeve auth login`.
pub struct Resolved {
    pub url: String,
    pub token: String,
}
pub fn resolve() -> anyhow::Result<Resolved> {
    let c = load();
    let url = env_nonempty("REEVE_URL").or(c.url).ok_or_else(|| {
        anyhow::anyhow!(
            "no server url: set REEVE_URL or run `reeve auth set-token --url ... --token ...`"
        )
    })?;
    let token = env_nonempty("REEVE_TOKEN").or(c.token).ok_or_else(|| {
        anyhow::anyhow!("no token: set REEVE_TOKEN or run `reeve auth set-token`")
    })?;
    Ok(Resolved {
        url: url.trim_end_matches('/').to_string(),
        token,
    })
}
