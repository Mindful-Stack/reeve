//! Wire types for the Reeve planner API and its MCP tools. No behaviour beyond key parsing, the conversion of a
//! DTO into its summary (`mcp.rs`), the item link name table (`links.rs`) and the few text rules every crate must
//! apply alike: how an email compares (`normalise_email`), what a target month is (`is_target_month`) and how much
//! of a caller's value a refusal repeats (`cut`, `echo`).
pub mod activity;
pub mod attachments;
pub mod cli_dist;
pub mod connections;
pub mod context;
pub mod enums;
pub mod envelope;
pub mod export;
pub mod findings;
pub mod identity;
pub mod ids;
pub mod items;
pub mod links;
pub mod mcp;
pub mod plan;
pub mod promise;
pub mod readiness;
pub mod requests;
pub mod sprints;
pub mod templates;
pub mod urgency;
pub use activity::*;
pub use attachments::*;
pub use cli_dist::*;
pub use connections::*;
pub use context::*;
pub use enums::*;
pub use envelope::*;
pub use export::*;
pub use findings::*;
pub use identity::*;
pub use ids::*;
pub use items::*;
pub use links::*;
pub use mcp::*;
pub use plan::*;
pub use promise::*;
pub use readiness::*;
pub use requests::*;
pub use sprints::*;
pub use templates::*;
pub use urgency::*;

/// The one rule for comparing people by email across every crate: trimmed and ASCII-lowercased. Identity, planning
/// (owners stored and filtered) and the CLI (`--mine`, decider matching) all call this.
pub fn normalise_email(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// How much of a caller's own value a refusal repeats, in characters (`echo`).
pub const MAX_ECHO: usize = 200;

/// Cuts `text` to its first `limit` characters when it has more, and appends `[cut: <n> more characters]`, n being
/// the number of characters removed. Characters, not bytes: the cut is always on a character boundary. The one cut
/// and the one marker: `echo` is this cut at `MAX_ECHO`, and the server's MCP adapter cuts long text with it at its
/// own limits.
pub fn cut(text: &mut String, limit: usize) {
    // The byte offset of the character after the last one kept; `None` when the text is within the limit.
    if let Some((at, _)) = text.char_indices().nth(limit) {
        let removed = text[at..].chars().count();
        text.truncate(at);
        text.push_str(&format!("[cut: {removed} more characters]"));
    }
}

/// A caller's value as a refusal repeats it: its first `MAX_ECHO` characters, and how many were left out, so the
/// words after it are still said however long it is. Every refusal that repeats a caller's value calls this one:
/// the server's actions and tool adapter (through its `text` module) and planning's domain rules
/// (`named_target_month`), whose messages the API sends as they are.
pub fn echo(text: &str) -> String {
    let mut text = text.to_string();
    cut(&mut text, MAX_ECHO);
    text
}

#[cfg(test)]
mod tests {
    use super::{MAX_ECHO, echo, normalise_email};

    #[test]
    fn emails_are_trimmed_and_ascii_lowercased() {
        assert_eq!(normalise_email(" Bob@Example.COM "), "bob@example.com");
    }

    #[test]
    fn an_echoed_argument_is_at_most_two_hundred_characters() {
        assert_eq!(echo("TST-12"), "TST-12");
        assert_eq!(echo(&"å".repeat(MAX_ECHO)), "å".repeat(MAX_ECHO));
        assert_eq!(
            echo(&"å".repeat(MAX_ECHO + 5)),
            format!("{}[cut: 5 more characters]", "å".repeat(MAX_ECHO))
        );
    }
}
