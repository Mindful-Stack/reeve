//! Opening a URL in the person's browser: `open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows (no shell,
//! so `&` in a URL is never a command separator), `xdg-open` elsewhere.

/// The program and its arguments for `os` (`std::env::consts::OS`).
pub(crate) fn argv(os: &str, url: &str) -> Vec<String> {
    let pre: &[&str] = match os {
        "macos" => &["open"],
        "windows" => &["rundll32", "url.dll,FileProtocolHandler"],
        _ => &["xdg-open"],
    };
    pre.iter()
        .map(|s| s.to_string())
        .chain([url.to_string()])
        .collect()
}

/// The opener for this platform, with its standard streams closed.
pub(crate) fn command(url: &str) -> std::process::Command {
    let a = argv(std::env::consts::OS, url);
    let mut c = std::process::Command::new(&a[0]);
    c.args(&a[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_platform_has_its_opener() {
        let u = "https://reeve.example.com/tokens?for=cli&x=1";
        assert_eq!(argv("macos", u), ["open", u]);
        assert_eq!(argv("linux", u), ["xdg-open", u]);
        // Not `cmd /C start`: cmd would split the URL at `&`.
        assert_eq!(
            argv("windows", u),
            ["rundll32", "url.dll,FileProtocolHandler", u]
        );
    }
}
