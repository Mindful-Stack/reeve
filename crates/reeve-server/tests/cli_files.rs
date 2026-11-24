//! `reeve_protocol::CLI_BINARIES` (what the server serves and the CLI downloads) and `scripts/cli-build.sh` (what the
//! release builds) are two copies of one list of file names; this keeps them equal (kb/ADR-0030 R3).
use std::{path::Path, process::Command};

#[test]
fn cli_binaries_are_the_files_the_release_builds() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/cli-build.sh");
    let run = |args: &[&str]| {
        let out = Command::new("bash")
            .arg(&script)
            .args(args)
            .output()
            .expect("bash runs");
        assert!(
            out.status.success(),
            "cli-build.sh {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8")
    };
    let mut built: Vec<String> = run(&["--targets"])
        .lines()
        .map(|t| run(&["--file", t]).trim().to_string())
        .collect();
    built.sort();
    let mut served: Vec<String> = reeve_protocol::CLI_BINARIES
        .iter()
        .map(|f| f.to_string())
        .collect();
    served.sort();
    assert_eq!(served, built);
}
