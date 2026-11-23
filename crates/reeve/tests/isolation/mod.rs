//! Proxy isolation shared by every test crate that runs the `reeve` binary. A developer's (or CI's) `HTTP_PROXY`
//! would otherwise receive the requests meant for an in-test server, token included.
#![allow(dead_code)] // each test file uses part of it

/// Every variable a proxy can be configured through.
pub const PROXY_VARS: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// The `reeve` binary, run without any proxy variable.
pub fn reeve_cmd() -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("reeve").unwrap();
    for v in PROXY_VARS {
        c.env_remove(v);
    }
    c
}

/// `c` without any proxy variable, and killed if the test drops it (a timed-out test leaves no child behind).
pub fn isolated(c: &mut tokio::process::Command) -> &mut tokio::process::Command {
    for v in PROXY_VARS {
        c.env_remove(v);
    }
    c.kill_on_drop(true)
}
