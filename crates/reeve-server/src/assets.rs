//! The web client, embedded from `web/build` (built by `mise run web`) with an SPA fallback and a content security policy.
//! Debug builds can read a directory instead (`REEVE_ASSETS_DIR`, and the tests); release builds serve only what is embedded.
use crate::{error::AppError, state::AppState};
use axum::{
    extract::State,
    http::{
        HeaderValue, Method, StatusCode, Uri,
        header::{
            CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY,
            X_CONTENT_TYPE_OPTIONS,
        },
    },
    response::{IntoResponse, Response},
};
use base64::Engine;
use sha2::{Digest, Sha256};
#[cfg(not(debug_assertions))]
use std::sync::OnceLock;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/build"]
struct Embedded;

#[derive(Clone)]
pub enum Assets {
    Embedded,
    /// For tests and `REEVE_ASSETS_DIR` in development; compiled out of release builds.
    #[cfg(debug_assertions)]
    Dir(std::path::PathBuf),
}

impl Assets {
    /// The bytes and content type of one file; `None` when it does not exist (or is a directory).
    fn file(&self, path: &str) -> Option<(Vec<u8>, mime_guess::Mime)> {
        let bytes = match self {
            Assets::Embedded => Embedded::get(path)?.data.into_owned(),
            #[cfg(debug_assertions)]
            Assets::Dir(root) => {
                let full = root.join(path);
                if !full.is_file() {
                    return None;
                }
                std::fs::read(full).ok()?
            }
        };
        Some((bytes, mime_guess::from_path(path).first_or_octet_stream()))
    }

    /// The `Content-Security-Policy` for the page: our own origin plus one `'sha256-…'` per inline `<script>` body in
    /// `index.html` (SvelteKit's static adapter emits an inline bootstrap script; the header must come from the server,
    /// so `kit.csp` is not used). Computed once for the release build, per request in debug.
    pub fn csp(&self) -> String {
        // Release embeds the build, so the policy cannot change: compute it once. Debug builds read the files from disk
        // (rust-embed without `debug-embed`, or the directory override), so recompute per request; a rebuilt client
        // must not meet a stale hash.
        #[cfg(not(debug_assertions))]
        {
            static EMBEDDED: OnceLock<String> = OnceLock::new();
            EMBEDDED.get_or_init(|| self.compute_csp()).clone()
        }
        #[cfg(debug_assertions)]
        {
            self.compute_csp()
        }
    }

    fn compute_csp(&self) -> String {
        let html = self
            .file("index.html")
            .map(|(b, _)| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let hashes: String = inline_script_bodies(&html)
            .iter()
            .map(|body| {
                let digest = Sha256::digest(body.as_bytes());
                format!(
                    " 'sha256-{}'",
                    base64::engine::general_purpose::STANDARD.encode(digest)
                )
            })
            .collect();
        format!(
            "default-src 'self'; script-src 'self'{hashes}; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'"
        )
    }
}

/// The non-empty bodies of `<script>` elements without a `src` attribute, exactly as written (CSP hashes the raw text).
fn inline_script_bodies(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase(); // same byte offsets as `html`
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = lower[at..].find("<script") {
        let tag_start = at + open;
        let Some(tag_len) = lower[tag_start..].find('>') else {
            break;
        };
        let body_start = tag_start + tag_len + 1;
        let Some(close) = lower[body_start..].find("</script") else {
            break;
        };
        let tag = &lower[tag_start..body_start];
        let body = &html[body_start..body_start + close];
        if !tag.contains("src=") && !body.is_empty() {
            out.push(body);
        }
        at = body_start + close;
    }
    out
}

fn content_type(mime: &mime_guess::Mime) -> String {
    let essence = mime.essence_str();
    if mime.type_() == mime_guess::mime::TEXT
        || essence.ends_with("javascript")
        || essence.ends_with("json")
    {
        format!("{essence}; charset=utf-8")
    } else {
        essence.to_string()
    }
}

/// Every served file and page: the browser never sniffs a type other than the one sent (`nosniff`), and following a link
/// out of the client sends no `Referer` (`no-referrer`), so a board or item URL never leaks to a linked site.
fn hardened(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    res
}

/// The router's fallback: a file when one matches, otherwise `index.html` so client-side routes work on reload.
pub async fn serve(State(s): State<AppState>, method: Method, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // API paths and the browser endpoints are never pages (`/auth/status` was added alongside login, callback
    // and logout); everything else, including the client page `/auth/signin`, may fall back to index.html.
    let trimmed = path.trim_end_matches('/');
    if trimmed == "api"
        || path.starts_with("api/")
        || trimmed == "health"
        || matches!(
            trimmed,
            "auth/login" | "auth/callback" | "auth/logout" | "auth/status"
        )
    {
        return AppError::NotFound.into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return AppError::MethodNotAllowed.into_response();
    }
    // Reject traversal before touching the filesystem (hyper does not normalise `..`); rust-embed checks this itself,
    // the directory override must do it here.
    if path.split('/').any(|seg| seg == "..") || path.contains('\\') || path.contains('\0') {
        return AppError::NotFound.into_response();
    }
    if let Some((bytes, mime)) = s.assets.file(path) {
        let cache = if path.starts_with("_app/immutable/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        let mut res = (
            [
                (CONTENT_TYPE, content_type(&mime)),
                (CACHE_CONTROL, cache.into()),
            ],
            bytes,
        )
            .into_response();
        // every HTML document carries the policy, not only the fallback (`/index.html` is reachable by name)
        if mime.essence_str() == "text/html"
            && let Ok(v) = s.assets.csp().parse()
        {
            res.headers_mut().insert(CONTENT_SECURITY_POLICY, v);
        }
        return hardened(res);
    }
    // Only page paths fall back: no extension in the last segment and not under `_app/`. A missing script, icon or
    // stale chunk is a 404, not an HTML page that the browser then fails to run.
    let last = path.rsplit('/').next().unwrap_or("");
    if last.contains('.') || path == "_app" || path.starts_with("_app/") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match s.assets.file("index.html") {
        Some((bytes, _)) => hardened(
            (
                [
                    (CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                    (CACHE_CONTROL, "no-cache".to_string()),
                    (CONTENT_SECURITY_POLICY, s.assets.csp()),
                ],
                bytes,
            )
                .into_response(),
        ),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "client not built: run `mise run web` (see deploy/README.md)",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::inline_script_bodies;

    #[test]
    fn only_inline_script_bodies_are_collected() {
        let html = r#"<script src="/a.js"></script><SCRIPT>window.x=1</SCRIPT><script type="module">import("/b.js")</script><script></script>"#;
        assert_eq!(
            inline_script_bodies(html),
            vec!["window.x=1", r#"import("/b.js")"#]
        );
    }
}
