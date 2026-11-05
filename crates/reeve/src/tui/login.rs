//! The TUI's credential (kb/ADR-0011 R7): the stored login only. `REEVE_TOKEN` is the automation credential every other
//! command prefers (`config::resolve`); the TUI is a surface where a person acts as themselves, so it never uses it and
//! only notes that it is set. The URL follows the CLI's rule (`REEVE_URL`, then the stored url).
use crate::config::{CliConfig, env_nonempty, load};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub url: String,
    pub token: String,
    /// `REEVE_TOKEN` is set and non-empty: shown as "REEVE_TOKEN ignored".
    pub reeve_token_set: bool,
}

pub(crate) fn resolve() -> anyhow::Result<Login> {
    resolve_from(load(), env_nonempty)
}

pub(crate) fn resolve_from(
    stored: CliConfig,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Login> {
    let nonempty = |k: &str| env(k).filter(|v| !v.is_empty());
    let url = nonempty("REEVE_URL")
        .or(stored.url)
        .ok_or_else(|| anyhow::anyhow!("no server url: run reeve auth login"))?;
    let token = stored.token.filter(|t| !t.is_empty()).ok_or_else(|| {
        anyhow::anyhow!(
            "no stored login: run reeve auth login (REEVE_TOKEN is not used by the TUI)"
        )
    })?;
    Ok(Login {
        url: url.trim_end_matches('/').to_string(),
        token,
        reeve_token_set: nonempty("REEVE_TOKEN").is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CliConfig;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_stored_token_is_the_only_credential() {
        let stored = CliConfig {
            url: Some("https://r.example.com/".into()),
            token: Some("stored".into()),
        };
        let l = resolve_from(stored, env(&[("REEVE_TOKEN", "from-env")])).unwrap();
        assert_eq!(l.token, "stored");
        assert_eq!(l.url, "https://r.example.com");
        assert!(l.reeve_token_set);
    }

    #[test]
    fn reeve_token_alone_is_not_a_login() {
        let e = resolve_from(
            CliConfig {
                url: Some("https://r".into()),
                token: None,
            },
            env(&[("REEVE_TOKEN", "x")]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("reeve auth login"), "{e}");
    }

    #[test]
    fn reeve_url_wins_over_the_stored_url_and_an_empty_token_variable_is_unset() {
        let stored = CliConfig {
            url: Some("https://stored".into()),
            token: Some("t".into()),
        };
        let l = resolve_from(
            stored,
            env(&[("REEVE_URL", "https://env/"), ("REEVE_TOKEN", "")]),
        )
        .unwrap();
        assert_eq!(l.url, "https://env");
        assert!(!l.reeve_token_set);
    }

    #[test]
    fn no_url_is_an_error() {
        let e = resolve_from(
            CliConfig {
                url: None,
                token: Some("t".into()),
            },
            env(&[]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("reeve auth login"), "{e}");
    }
}
