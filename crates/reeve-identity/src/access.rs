//! Who may sign in and who is admin: the configured floor ∪ the identity tables (kb/ADR-0015 R2, R4). Pure: the
//! store loads the tables (`access_repo`), the handlers decide on it (`access_handler`).
use crate::domain::IdentityConfig;
use chrono::{DateTime, Utc};
use reeve_protocol::{Role, normalise_email};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessKind {
    Domain,
    Email,
}

impl AccessKind {
    /// The stored spelling (`allowed_entries.kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Email => "email",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "domain" => Some(Self::Domain),
            "email" => Some(Self::Email),
            _ => None,
        }
    }
}

/// One database row: an allowed domain or email, or an admin grant (`value` is then the email).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGrant {
    pub value: String,
    pub added_by: String,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessSource {
    Config,
    Database {
        added_by: String,
        added_at: DateTime<Utc>,
    },
}

/// One listed entry. `also_in_config` marks a database row that duplicates a floor entry (kb/ADR-0015 R2); `allowed`
/// is false only on a database admin grant whose email nothing allows (an inert grant, kb/ADR-0015 R4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRow {
    pub value: String,
    pub source: AccessSource,
    pub also_in_config: bool,
    pub allowed: bool,
}

/// Every entry, one row per (value, source): floor rows first, then database rows, each group by value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessView {
    pub domains: Vec<AccessRow>,
    pub emails: Vec<AccessRow>,
    pub admins: Vec<AccessRow>,
}

/// The floor and the tables, normalised (`normalise_email`) and sorted on construction.
#[derive(Debug, Clone)]
pub struct AccessPolicy {
    floor_domains: Vec<String>,
    floor_emails: Vec<String>,
    floor_admins: Vec<String>,
    domains: Vec<AccessGrant>,
    emails: Vec<AccessGrant>,
    admins: Vec<AccessGrant>,
}

fn norm_list(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = v
        .iter()
        .map(|s| normalise_email(s))
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}
fn norm_grants(mut v: Vec<AccessGrant>) -> Vec<AccessGrant> {
    for g in &mut v {
        g.value = normalise_email(&g.value);
    }
    v.sort_by(|a, b| a.value.cmp(&b.value));
    v
}

impl AccessPolicy {
    pub fn new(
        floor: &IdentityConfig,
        domains: Vec<AccessGrant>,
        emails: Vec<AccessGrant>,
        admins: Vec<AccessGrant>,
    ) -> Self {
        Self {
            floor_domains: norm_list(&floor.allowed_domains),
            floor_emails: norm_list(&floor.allowed_emails),
            floor_admins: norm_list(&floor.admins),
            domains: norm_grants(domains),
            emails: norm_grants(emails),
            admins: norm_grants(admins),
        }
    }
    /// The floor alone: the operator's commands before the store exists, and tests.
    pub fn floor_only(floor: &IdentityConfig) -> Self {
        Self::new(floor, vec![], vec![], vec![])
    }
    fn floor(&self, kind: AccessKind) -> &[String] {
        match kind {
            AccessKind::Domain => &self.floor_domains,
            AccessKind::Email => &self.floor_emails,
        }
    }
    fn table(&self, kind: AccessKind) -> &[AccessGrant] {
        match kind {
            AccessKind::Domain => &self.domains,
            AccessKind::Email => &self.emails,
        }
    }
    /// Same split as `IdentityConfig::is_allowed`: one `@`, non-empty local part and domain.
    pub fn is_allowed(&self, email: &str) -> bool {
        let email = normalise_email(email);
        let Some((local, domain)) = email.split_once('@') else {
            return false;
        };
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return false;
        }
        self.floor_emails.contains(&email)
            || self.emails.iter().any(|g| g.value == email)
            || self.floor_domains.iter().any(|d| d == domain)
            || self.domains.iter().any(|g| g.value == domain)
    }
    /// A floor admin is admin; a database grant only while its email is allowed (kb/ADR-0015 R4).
    pub fn role_for(&self, email: &str) -> Role {
        let email = normalise_email(email);
        if self.floor_admins.contains(&email)
            || (self.admins.iter().any(|g| g.value == email) && self.is_allowed(&email))
        {
            Role::Admin
        } else {
            Role::Member
        }
    }
    pub fn in_floor(&self, kind: AccessKind, value: &str) -> bool {
        self.floor(kind).contains(&normalise_email(value))
    }
    pub fn in_table(&self, kind: AccessKind, value: &str) -> bool {
        let v = normalise_email(value);
        self.table(kind).iter().any(|g| g.value == v)
    }
    pub fn is_floor_admin(&self, email: &str) -> bool {
        self.floor_admins.contains(&normalise_email(email))
    }
    pub fn has_admin_grant(&self, email: &str) -> bool {
        let e = normalise_email(email);
        self.admins.iter().any(|g| g.value == e)
    }
    pub fn admin_grants(&self) -> &[AccessGrant] {
        &self.admins
    }
    /// This policy with the database row `(kind, value)` removed; the floor is never touched.
    pub fn without_entry(&self, kind: AccessKind, value: &str) -> Self {
        let v = normalise_email(value);
        let mut p = self.clone();
        match kind {
            AccessKind::Domain => p.domains.retain(|g| g.value != v),
            AccessKind::Email => p.emails.retain(|g| g.value != v),
        }
        p
    }
    pub fn view(&self) -> AccessView {
        fn rows(
            floor: &[String],
            table: &[AccessGrant],
            allowed: &dyn Fn(&str) -> bool,
        ) -> Vec<AccessRow> {
            let mut out: Vec<AccessRow> = floor
                .iter()
                .map(|v| AccessRow {
                    value: v.clone(),
                    source: AccessSource::Config,
                    also_in_config: false,
                    allowed: true,
                })
                .collect();
            out.extend(table.iter().map(|g| AccessRow {
                value: g.value.clone(),
                source: AccessSource::Database {
                    added_by: g.added_by.clone(),
                    added_at: g.added_at,
                },
                also_in_config: floor.contains(&g.value),
                allowed: allowed(&g.value),
            }));
            out
        }
        AccessView {
            domains: rows(&self.floor_domains, &self.domains, &|_| true),
            emails: rows(&self.floor_emails, &self.emails, &|_| true),
            admins: rows(&self.floor_admins, &self.admins, &|e| self.is_allowed(e)),
        }
    }
}

/// The longest stored value (an email address is at most 254 characters); a longer one overflows the btree key.
const MAX_VALUE_BYTES: usize = 254;

fn too_long(what: &str) -> String {
    format!("the {what} is too long (at most {MAX_VALUE_BYTES} characters)")
}

/// A domain an admin may allow: non-empty, contains a `.`, no `@`, no whitespace or control character, at most 254 bytes. Returns it normalised.
pub fn parse_domain(raw: &str) -> Result<String, String> {
    let v = normalise_email(raw);
    if v.len() > MAX_VALUE_BYTES {
        return Err(too_long("domain"));
    }
    if v.is_empty()
        || !v.contains('.')
        || v.contains('@')
        || v.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("{:?} is not a domain", raw.trim()));
    }
    Ok(v)
}

/// An email an admin may allow: one `@`, non-empty local part and domain, no whitespace or control character, at most 254 bytes. Returns it normalised.
pub fn parse_email(raw: &str) -> Result<String, String> {
    let v = normalise_email(raw);
    if v.len() > MAX_VALUE_BYTES {
        return Err(too_long("email address"));
    }
    let ok = match v.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && !domain.contains('@'),
        None => false,
    };
    if !ok || v.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("{:?} is not an email address", raw.trim()));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0).unwrap()
    }
    fn g(v: &str) -> AccessGrant {
        AccessGrant {
            value: v.into(),
            added_by: "dan@example.com".into(),
            added_at: at(),
        }
    }
    fn floor() -> IdentityConfig {
        IdentityConfig {
            allowed_domains: vec!["Example.com".into()],
            allowed_emails: vec!["svc@bots.dev".into()],
            admins: vec!["dan@example.com".into()],
        }
    }

    #[test]
    fn control_characters_and_overlong_values_are_refused() {
        assert!(parse_email("a\u{0}@x.org").is_err());
        assert!(parse_domain("x\u{7}.org").is_err());
        let long_email = format!("{}@x.org", "a".repeat(255 - 6));
        assert_eq!(long_email.len(), 255);
        let e = parse_email(&long_email).unwrap_err();
        assert!(e.contains("too long (at most 254 characters)") && e.len() < 100);
        assert!(parse_email(&format!("{}@x.org", "a".repeat(254 - 6))).is_ok());
        let long_domain = format!("{}.org", "a".repeat(252));
        assert_eq!(long_domain.len(), 256);
        assert!(parse_domain(&long_domain).is_err());
        assert!(parse_domain(&format!("{}.org", "a".repeat(250))).is_ok());
    }

    #[test]
    fn the_floor_and_each_table_allow_and_nothing_else_does() {
        let p = AccessPolicy::new(
            &floor(),
            vec![g("partner.org")],
            vec![g(" Guest@Other.org ")],
            vec![],
        );
        assert!(p.is_allowed("ada@EXAMPLE.com")); // floor domain, case-insensitive
        assert!(p.is_allowed("svc@bots.dev")); // floor email
        assert!(p.is_allowed("x@partner.org")); // table domain
        assert!(p.is_allowed("guest@other.org")); // table email, stored un-normalised
        assert!(!p.is_allowed("x@other.org"));
        assert!(!p.is_allowed("example.com"));
        assert!(!p.is_allowed("a@b@example.com"));
        assert!(!p.is_allowed("@example.com"));
    }

    #[test]
    fn a_grant_makes_admin_only_while_its_email_is_allowed() {
        let p = AccessPolicy::new(
            &floor(),
            vec![],
            vec![],
            vec![g("ada@example.com"), g("eve@gone.org")],
        );
        assert_eq!(p.role_for("dan@example.com"), Role::Admin); // floor admin
        assert_eq!(p.role_for("ada@example.com"), Role::Admin); // grant, allowed by the floor domain
        assert_eq!(p.role_for("eve@gone.org"), Role::Member); // inert: nothing allows gone.org
        assert_eq!(p.role_for("bob@example.com"), Role::Member);
        assert!(p.has_admin_grant("eve@gone.org"));
    }

    #[test]
    fn without_entry_drops_only_the_database_row() {
        let p = AccessPolicy::new(
            &floor(),
            vec![g("example.com")],
            vec![g("guest@other.org")],
            vec![],
        );
        let q = p.without_entry(AccessKind::Domain, "EXAMPLE.com");
        assert!(!q.in_table(AccessKind::Domain, "example.com"));
        assert!(q.in_floor(AccessKind::Domain, "example.com"));
        assert!(q.is_allowed("ada@example.com"), "the floor still allows it");
        let r = p.without_entry(AccessKind::Email, "guest@other.org");
        assert!(!r.is_allowed("guest@other.org"));
        assert!(p.is_allowed("guest@other.org"), "the original is unchanged");
    }

    #[test]
    fn the_view_lists_each_source_floor_first_and_flags_duplicates_and_inert_grants() {
        let p = AccessPolicy::new(
            &floor(),
            vec![g("example.com"), g("acme.io")],
            vec![],
            vec![g("eve@gone.org"), g("ada@example.com")],
        );
        let v = p.view();
        let domains: Vec<_> = v
            .domains
            .iter()
            .map(|r| {
                (
                    r.value.as_str(),
                    matches!(r.source, AccessSource::Config),
                    r.also_in_config,
                )
            })
            .collect();
        assert_eq!(
            domains,
            [
                ("example.com", true, false),
                ("acme.io", false, false),
                ("example.com", false, true)
            ]
        );
        let admins: Vec<_> = v
            .admins
            .iter()
            .map(|r| (r.value.as_str(), r.allowed))
            .collect();
        assert_eq!(
            admins,
            [
                ("dan@example.com", true),
                ("ada@example.com", true),
                ("eve@gone.org", false)
            ]
        );
        assert!(v.emails.iter().all(|r| r.allowed));
    }

    #[test]
    fn domains_match_exactly_never_by_suffix() {
        let p = AccessPolicy::new(
            &IdentityConfig {
                allowed_domains: vec!["example.com".into()],
                allowed_emails: vec![],
                admins: vec![],
            },
            vec![g("partner.org")],
            vec![],
            vec![],
        );
        for bad in [
            "a@evil-example.com",
            "a@sub.example.com",
            "a@example.com.evil",
            "a@sub.partner.org",
        ] {
            assert!(!p.is_allowed(bad), "{bad}");
        }
        assert!(p.is_allowed("a@example.com") && p.is_allowed("a@partner.org"));
    }

    #[test]
    fn a_floor_only_policy_answers_like_the_configuration() {
        let cfg = IdentityConfig {
            allowed_domains: vec!["Example.com".into(), "  ".into(), "".into()],
            allowed_emails: vec![" Guest@Other.org ".into(), "".into()],
            admins: vec!["Dan@Example.com".into()],
        };
        let p = AccessPolicy::floor_only(&cfg);
        for i in [
            "dan@example.com",
            " DAN@EXAMPLE.COM ",
            "guest@other.org",
            "x@other.org",
            "a@b@example.com",
            "@example.com",
            "example.com",
            "",
            " ",
        ] {
            assert_eq!(p.is_allowed(i), cfg.is_allowed(i), "is_allowed({i:?})");
            if !i.trim().is_empty() {
                assert_eq!(p.role_for(i), cfg.role_for(i), "role_for({i:?})");
            }
        }
    }

    #[test]
    fn floor_admin_rows_are_always_allowed() {
        let cfg = IdentityConfig {
            allowed_domains: vec!["example.com".into()],
            allowed_emails: vec![],
            admins: vec!["boss@other.org".into()],
        };
        let v = AccessPolicy::new(&cfg, vec![], vec![], vec![]).view();
        assert_eq!(v.admins.len(), 1);
        assert!(v.admins[0].allowed);
    }

    #[test]
    fn domains_and_emails_are_validated_and_normalised() {
        assert_eq!(parse_domain(" Partner.ORG ").unwrap(), "partner.org");
        for bad in ["", "partner", "a@partner.org", "part ner.org"] {
            assert!(parse_domain(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_email(" Guest@Other.org ").unwrap(), "guest@other.org");
        for bad in [
            "",
            "guest",
            "@other.org",
            "guest@",
            "a@b@c.org",
            "gu est@other.org",
        ] {
            assert!(parse_email(bad).is_err(), "{bad:?}");
        }
    }
}
