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

/// The opener for this platform, with its standard streams closed. Only `http` and `https` URLs are opened: anything
/// else (a `file:` URL, a bare path, a program name) would make the opener run or show something that is not a page.
///
/// On Windows `rundll32` never reports failure: it exits 0 whether or not anything opened, so a success status there
/// proves nothing.
pub(crate) fn command(url: &str) -> Result<std::process::Command, String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("refusing to open a url that is not http or https".into());
    }
    let a = argv(std::env::consts::OS, url);
    let mut c = std::process::Command::new(&a[0]);
    c.args(&a[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    Ok(c)
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

    #[test]
    fn only_http_and_https_urls_are_opened() {
        assert!(command("https://reeve.example.com/x?a=1&b=2").is_ok());
        assert!(command("HTTP://127.0.0.1:3000/").is_ok());
        for bad in [
            "file:///etc/passwd",
            "calc.exe",
            "javascript:alert(1)",
            "",
            "ftp://x",
            "-h",
        ] {
            assert!(command(bad).is_err(), "{bad}");
        }
    }
}
