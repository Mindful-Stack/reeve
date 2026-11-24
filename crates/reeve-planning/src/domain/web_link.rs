//! Web links (kb/ADR-0031 R5): a titled URL on one item, in a list of its own, apart from item links. Pure: it
//! belongs to its one item, is part of nothing else, and is no readiness fact (R3).

/// A title is at most this many characters, after trimming.
pub const MAX_TITLE: usize = 200;
/// A stored URL is at most this many characters. The MCP read tools never cut a URL (a cut URL is a wrong URL),
/// so this limit is what keeps ten of them small.
pub const MAX_URL: usize = 1_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebLinkError {
    #[error("a web link's title must be 1 to 200 characters")]
    Title,
    #[error(
        "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password"
    )]
    Url,
}

/// A title and a URL, both as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebLink {
    title: String,
    url: String,
}

impl WebLink {
    /// The title trimmed, 1 to `MAX_TITLE` characters; the URL as `canonical_url` gives it.
    pub fn new(title: &str, url: &str) -> Result<Self, WebLinkError> {
        let title = title.trim();
        if !(1..=MAX_TITLE).contains(&title.chars().count()) {
            return Err(WebLinkError::Title);
        }
        Ok(Self {
            title: title.to_string(),
            url: canonical_url(url)?,
        })
    }
    /// A stored row, as it is: no check.
    pub fn from_persisted(title: String, url: String) -> Self {
        Self { title, url }
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// The form a URL is stored and compared in: the `url` crate's serialisation of the trimmed input, so two
/// spellings of one address (`HTTP://Example.com`, `http://example.com/`) are the same URL. Refused: anything the
/// parser rejects, a scheme other than `http` or `https` (the web client renders it as a link, and a
/// `javascript:` URL would run there), no host, a user name or a password (a credential would be shown to every
/// reader and kept by the activity log after the web link is gone), and a serialisation over `MAX_URL` characters.
pub fn canonical_url(raw: &str) -> Result<String, WebLinkError> {
    let parsed = url::Url::parse(raw.trim()).map_err(|_| WebLinkError::Url)?;
    let web = matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|h| !h.is_empty());
    // As `check_client_id` in reeve-identity: any user information is refused, not stripped.
    let credentials = !parsed.username().is_empty() || parsed.password().is_some();
    let stored = String::from(parsed);
    if !web || credentials || stored.chars().count() > MAX_URL {
        return Err(WebLinkError::Url);
    }
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_trimmed_and_is_one_to_two_hundred_characters() {
        let link = WebLink::new("  Scenario page ", "https://example.com/wiki").unwrap();
        assert_eq!(link.title(), "Scenario page");
        // Characters, not bytes: 200 two-byte characters are a title.
        assert!(WebLink::new(&"å".repeat(200), "https://example.com/").is_ok());
        for bad in [String::new(), "   ".to_string(), "a".repeat(201)] {
            assert_eq!(
                WebLink::new(&bad, "https://example.com/"),
                Err(WebLinkError::Title),
                "{} characters",
                bad.chars().count()
            );
        }
    }

    /// Sameness is equality of the stored form. Each pair is two spellings of one address.
    #[test]
    fn a_url_is_stored_as_the_parser_writes_it_so_two_spellings_are_the_same_url() {
        for (typed, stored) in [
            (
                "https://example.com/wiki/scenario",
                "https://example.com/wiki/scenario",
            ),
            ("  HTTP://Example.COM  ", "http://example.com/"),
            ("https://example.com:443/a", "https://example.com/a"),
            ("http://example.com/a b", "http://example.com/a%20b"),
        ] {
            assert_eq!(canonical_url(typed).as_deref(), Ok(stored), "{typed}");
            assert_eq!(
                canonical_url(stored).as_deref(),
                Ok(stored),
                "the stored form is stable"
            );
        }
        // Different addresses stay different: a path and a query are part of the URL.
        assert_ne!(
            canonical_url("https://example.com/a"),
            canonical_url("https://example.com/a?b=1")
        );
        assert_eq!(
            WebLink::new("T", "HTTPS://EXAMPLE.com/Wiki").unwrap().url(),
            "https://example.com/Wiki",
            "the host is lowercased, the path is not"
        );
    }

    #[test]
    fn only_absolute_http_and_https_urls_of_at_most_a_thousand_characters_are_taken() {
        for bad in [
            "",
            "example.com/wiki",
            "/wiki/scenario",
            "ftp://example.com/file",
            "javascript:alert(1)",
            "mailto:mia@example.com",
            "file:///etc/passwd",
            "http://",
        ] {
            assert_eq!(canonical_url(bad), Err(WebLinkError::Url), "{bad:?}");
        }
        let at_the_limit = format!("https://example.com/{}", "a".repeat(980));
        assert_eq!(at_the_limit.chars().count(), MAX_URL, "the control");
        assert_eq!(
            canonical_url(&at_the_limit).as_deref(),
            Ok(at_the_limit.as_str())
        );
        assert_eq!(
            canonical_url(&format!("{at_the_limit}a")),
            Err(WebLinkError::Url)
        );
        // The limit is on what is stored: this is 420 characters as typed and 2 420 once percent-encoded.
        let grows = format!("https://example.com/{}", "å".repeat(400));
        assert_eq!(canonical_url(&grows), Err(WebLinkError::Url));
    }

    /// A URL with user information is refused whole, never stored with the credential stripped: the person is told,
    /// and nothing of it reaches a row or an activity entry.
    #[test]
    fn a_url_with_a_user_name_or_a_password_is_refused() {
        // The control: the same address without user information is taken.
        assert_eq!(
            canonical_url("https://example.com/wiki").as_deref(),
            Ok("https://example.com/wiki")
        );
        for bad in [
            "https://mia:secret@example.com/wiki",
            "https://mia@example.com/wiki",
            "https://:secret@example.com/wiki",
            "HTTP://Mia:Secret@Example.com",
        ] {
            assert_eq!(canonical_url(bad), Err(WebLinkError::Url), "{bad}");
            assert_eq!(WebLink::new("Page", bad), Err(WebLinkError::Url), "{bad}");
        }
        // An `@` further along is not user information.
        assert_eq!(
            canonical_url("https://example.com/people/@mia?to=mia@example.com").as_deref(),
            Ok("https://example.com/people/@mia?to=mia@example.com")
        );
    }

    /// The wording a person reads when a title or a URL is refused.
    #[test]
    fn the_refusals_say_what_a_title_and_a_url_must_be() {
        assert_eq!(
            WebLinkError::Title.to_string(),
            "a web link's title must be 1 to 200 characters"
        );
        assert_eq!(
            WebLinkError::Url.to_string(),
            "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password"
        );
    }
}
