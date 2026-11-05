//! `reeve-server repos`: the scheduled check's list of repositories (`name<TAB>url`), read from the configuration alone.
use std::process::Output;

/// Runs the built binary on a configuration file holding `text`.
fn repos(text: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reeve.toml");
    std::fs::write(&path, text).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("--config")
        .arg(&path)
        .arg("repos")
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .unwrap()
}

/// Nothing listens on port 9 on loopback: a subcommand that connected to the database would fail.
const BASE: &str =
    "database_url = \"postgres://reeve:reeve@127.0.0.1:9/reeve\"\nkey_prefix = \"TST\"\n";

#[test]
fn prints_repositories_with_a_url_without_touching_the_database() {
    let out = repos(&format!(
        "{BASE}[[repos]]\nname = \"knowledge-base\"\nurl = \"git@github.com:G/knowledge-base.git\"\nadr_dir = \"knowledge/adrs\"\n[[repos]]\nname = \"portal\"\nadr_dir = \"docs/adr\"\n"
    ));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "knowledge-base\tgit@github.com:G/knowledge-base.git\n"
    );
    assert!(
        stderr.contains("warning: [[repos]] entry \"portal\" has no url"),
        "{stderr}"
    );
}

#[test]
fn a_bad_configuration_exits_1_with_the_parse_error() {
    let out = repos("database_url = \"postgres://x\"\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("key_prefix"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
}

#[test]
fn a_tab_in_a_name_exits_1_and_prints_nothing() {
    let out = repos(&format!(
        "{BASE}[[repos]]\nname = \"a\\tb\"\nurl = \"u\"\nadr_dir = \"x\"\n"
    ));
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}
