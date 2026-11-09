//! Wire types for the Reeve planner API. No behaviour beyond key parsing.
pub mod activity;
pub mod context;
pub mod enums;
pub mod envelope;
pub mod export;
pub mod findings;
pub mod identity;
pub mod ids;
pub mod items;
pub mod plan;
pub mod readiness;
pub mod requests;
pub mod sprints;
pub mod urgency;
pub use activity::*;
pub use context::*;
pub use enums::*;
pub use envelope::*;
pub use export::*;
pub use findings::*;
pub use identity::*;
pub use ids::*;
pub use items::*;
pub use plan::*;
pub use readiness::*;
pub use requests::*;
pub use sprints::*;
pub use urgency::*;

/// The one rule for comparing people by email across every crate: trimmed and ASCII-lowercased. Identity, planning
/// (owners stored and filtered) and the CLI (`--mine`, decider matching) all call this.
pub fn normalise_email(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::normalise_email;

    #[test]
    fn emails_are_trimmed_and_ascii_lowercased() {
        assert_eq!(normalise_email(" Bob@Example.COM "), "bob@example.com");
    }
}
