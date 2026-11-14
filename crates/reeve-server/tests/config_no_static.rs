//! A binary built without the `static-oidc` feature has no static provider at all: naming it is a parse error, not a
//! configuration that a loopback check might let through (kb/ADR-0015 R1). Run with
//! `cargo test -p reeve-server --test config_no_static` (no features); with the feature this file is empty.
#![cfg(not(feature = "static-oidc"))]

#[test]
fn the_static_provider_does_not_parse_without_the_feature() {
    let err = reeve_server::ServerConfig::parse(
        "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\n[oidc]\nprovider = \"static\"\nstatic_email = \"dan@example.com\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown variant"), "{err}");
    assert!(
        reeve_server::ServerConfig::parse(
            "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\npublic_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"i\"\nclient_secret = \"s\"\n",
        )
        .is_ok()
    );
}

#[test]
fn no_configuration_names_another_issuer() {
    let err = reeve_server::ServerConfig::parse(
        "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\npublic_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"i\"\nclient_secret = \"s\"\nissuer = \"http://127.0.0.1:9\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("issuer"), "{err}");
}

/// A production build has no way to mint an MCP token by hand (kb/ADR-0029 R5; GRA-658 adds the only real path). The
/// configuration path does not exist: a build that had the subcommand would fail on that instead, with status 1,
/// so a configuration error cannot pass for "no such subcommand".
#[test]
fn mcp_token_issue_does_not_exist_without_the_feature() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .args([
            "--config",
            "/nonexistent/reeve.toml",
            "mcp-token-issue",
            "--email",
            "dan@example.com",
        ])
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unrecognized subcommand"), "{stderr}");
    assert!(stderr.contains("mcp-token-issue"), "{stderr}");
}

/// A production build has no configuration under which a client document is fetched from loopback or a person is
/// sent back over http: the exception for both exists only with the static provider, which this build cannot
/// parse. This is what pins that production fetches through `make_client_fetcher` from real hosts only.
#[test]
fn a_production_build_refuses_a_loopback_client_host_and_an_http_redirect() {
    const BASE: &str = "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\npublic_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"i\"\nclient_secret = \"s\"\n[mcp]\nenabled = true\n";
    for (line, names) in [
        ("client_hosts = [\"127.0.0.1\"]", "[mcp] client_hosts"),
        (
            "redirect_uris = [\"http://127.0.0.1:6274/cb\"]",
            "[mcp] redirect_uris",
        ),
    ] {
        let err = reeve_server::ServerConfig::parse(&format!("{BASE}{line}\n")).unwrap_err();
        assert!(err.to_string().contains(names), "{line}: {err}");
    }
    // The control: the same configuration with real values loads.
    assert!(
        reeve_server::ServerConfig::parse(&format!(
            "{BASE}client_hosts = [\"claude.ai\"]\nredirect_uris = [\"https://claude.ai/api/mcp/auth_callback\"]\n"
        ))
        .is_ok()
    );
}
