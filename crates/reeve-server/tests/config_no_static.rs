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
