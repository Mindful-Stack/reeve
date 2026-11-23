//! The CLI distribution contract between the server and the CLI (kb/ADR-0030): where a bundle is served, the names of
//! its files, how its two text files are served, the response header that carries the build a server offers, and the
//! CLI-version format. Not an `/api/v1` type; the OpenAPI document does not carry it. Shell copies of the names live in
//! `deploy/cli/install.sh` (its platform map) and in `scripts/cli-build.sh`.

/// The response header carrying the CLI version of the bundle a server offers, while distribution is on. Lower case,
/// as HTTP/2 sends it.
pub const CLI_VERSION_HEADER: &str = "x-reeve-cli";

/// The path every file of a bundle is served under: `<server url>/cli/<name>`.
pub const CLI_PATH: &str = "/cli/";

/// The bundle's file holding the CLI version it offers: one CLI version, optionally followed by a newline.
pub const CLI_VERSION_FILE: &str = "VERSION";

/// The bundle's checksum file: one `<sha-256>  <name>` line per binary of `CLI_BINARIES`.
pub const CLI_SUMS_FILE: &str = "SHA256SUMS";

/// The shell installer, served at `/cli/install.sh`.
pub const CLI_INSTALL_SH: &str = "install.sh";

/// The PowerShell installer, served at `/cli/install.ps1`.
pub const CLI_INSTALL_PS1: &str = "install.ps1";

/// The media type `CLI_VERSION_FILE` and `CLI_SUMS_FILE` are served with (a `charset` parameter may follow). A `VERSION` served
/// as anything else is not a bundle's: an older server answers unknown paths with its web page.
pub const CLI_TEXT_CONTENT_TYPE: &str = "text/plain";

/// The binaries of a bundle, one per supported target, in `SHA256SUMS` order. A bundle is these plus `CLI_VERSION_FILE`
/// and `CLI_SUMS_FILE`.
pub const CLI_BINARIES: [&str; 5] = [
    "reeve-linux-x86_64",
    "reeve-linux-aarch64",
    "reeve-macos-aarch64",
    "reeve-macos-x86_64",
    "reeve-windows-x86_64.exe",
];

/// Whether `s` is a CLI version: a full commit id, exactly 40 lowercase hexadecimal characters.
pub fn is_cli_version(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinguishes a check that accepts a short id, upper case, a trailing newline (a `VERSION` file read whole) or a
    /// non-hex letter.
    #[test]
    fn a_cli_version_is_forty_lowercase_hex_characters() {
        assert!(is_cli_version("0123456789abcdef0123456789abcdef01234567"));
        for bad in [
            "",
            "0123456",
            "0123456789abcdef0123456789abcdef0123456",
            "0123456789abcdef0123456789abcdef012345678",
            "0123456789ABCDEF0123456789abcdef01234567",
            "0123456789abcdef0123456789abcdef0123456g",
            "0123456789abcdef0123456789abcdef01234567\n",
        ] {
            assert!(!is_cli_version(bad), "{bad:?}");
        }
    }

    /// The header name is a valid lower-case token (the server and the CLI build a `HeaderName` from it with
    /// `from_static`, which panics on upper case), and the names are distinct (from each other and from the two text
    /// files), one binary per platform, `.exe` only on Windows.
    #[test]
    fn the_names_are_well_formed() {
        assert!(
            CLI_VERSION_HEADER
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'-')
        );
        assert!(CLI_PATH.starts_with('/') && CLI_PATH.ends_with('/'));
        let mut seen = std::collections::HashSet::from([CLI_VERSION_FILE, CLI_SUMS_FILE]);
        for f in CLI_BINARIES {
            assert!(seen.insert(f), "{f} twice");
            let (os, arch) = f
                .strip_prefix("reeve-")
                .and_then(|r| r.trim_end_matches(".exe").split_once('-'))
                .unwrap_or_else(|| panic!("{f}"));
            assert!(matches!(os, "linux" | "macos" | "windows"), "{f}");
            assert!(matches!(arch, "x86_64" | "aarch64"), "{f}");
            assert_eq!(f.ends_with(".exe"), os == "windows", "{f}");
        }
    }
}
