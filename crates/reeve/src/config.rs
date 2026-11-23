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

/// The one way every command says there is no server to talk to (the prefix is what Lorekeeper's detector matches).
pub const NO_SERVER_URL: &str =
    "no server url: run reeve auth login --url https://reeve.example.com first, or set REEVE_URL";

/// The configuration directory: `REEVE_CONFIG_DIR`; else on Windows `%APPDATA%\reeve` (so PowerShell and Git Bash share
/// one file); else `$XDG_CONFIG_HOME/reeve`, then `$HOME/.config/reeve`.
pub(crate) fn config_dir_from(
    env: impl Fn(&str) -> Option<String>,
    windows: bool,
) -> anyhow::Result<PathBuf> {
    if let Some(d) = env("REEVE_CONFIG_DIR") {
        return Ok(PathBuf::from(d));
    }
    if windows {
        return env("APPDATA")
            .map(|a| PathBuf::from(a).join("reeve"))
            .ok_or_else(|| {
                anyhow::anyhow!("cannot locate a config directory: set REEVE_CONFIG_DIR or APPDATA")
            });
    }
    let base = match (env("XDG_CONFIG_HOME"), env("HOME")) {
        (Some(x), _) => PathBuf::from(x),
        (None, Some(h)) => PathBuf::from(h).join(".config"),
        (None, None) => anyhow::bail!(
            "cannot locate a config directory: set REEVE_CONFIG_DIR, XDG_CONFIG_HOME or HOME"
        ),
    };
    Ok(base.join("reeve"))
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir_from(env_nonempty, cfg!(windows))?.join("config.toml"))
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

/// Resolution order (kb/ADR-0015 R7): REEVE_TOKEN in the environment is an automation token and wins;
/// otherwise the interactive token stored by `reeve auth set-token` / plan B's `reeve auth login`.
pub struct Resolved {
    pub url: String,
    pub token: String,
}
pub fn resolve() -> anyhow::Result<Resolved> {
    let c = load();
    let url = env_nonempty("REEVE_URL")
        .or(c.url)
        .ok_or_else(|| anyhow::anyhow!(NO_SERVER_URL))?;
    let token = env_nonempty("REEVE_TOKEN").or(c.token).ok_or_else(|| {
        anyhow::anyhow!("no token: set REEVE_TOKEN or run `reeve auth set-token`")
    })?;
    Ok(Resolved {
        url: url.trim_end_matches('/').to_string(),
        token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned().filter(|v| !v.is_empty())
    }

    #[test]
    fn windows_uses_appdata_and_ignores_home() {
        let d = config_dir_from(
            env(&[
                ("APPDATA", r"C:\Users\d\AppData\Roaming"),
                ("HOME", "/c/Users/d"),
            ]),
            true,
        )
        .unwrap();
        assert_eq!(
            d,
            PathBuf::from(r"C:\Users\d\AppData\Roaming").join("reeve")
        );
    }

    #[test]
    fn reeve_config_dir_wins_everywhere() {
        for windows in [false, true] {
            let d = config_dir_from(
                env(&[
                    ("REEVE_CONFIG_DIR", "/x"),
                    ("APPDATA", "/a"),
                    ("HOME", "/h"),
                ]),
                windows,
            )
            .unwrap();
            assert_eq!(d, PathBuf::from("/x"), "windows={windows}");
        }
    }

    #[test]
    fn unix_is_unchanged() {
        assert_eq!(
            config_dir_from(env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]), false).unwrap(),
            PathBuf::from("/x/reeve")
        );
        assert_eq!(
            config_dir_from(env(&[("HOME", "/h"), ("APPDATA", "/a")]), false).unwrap(),
            PathBuf::from("/h/.config/reeve")
        );
    }

    #[test]
    fn nothing_to_go_on_is_an_error_naming_what_to_set() {
        let e = config_dir_from(env(&[]), true).unwrap_err().to_string();
        assert!(
            e.contains("REEVE_CONFIG_DIR") && e.contains("APPDATA"),
            "{e}"
        );
        let e = config_dir_from(env(&[]), false).unwrap_err().to_string();
        assert!(e.contains("HOME"), "{e}");
    }
}
