//! `reeve template`: brief templates, the named Markdown texts a new initiative's or epic's brief can start from
//! (`reeve initiative create --template <NAME>`, `reeve epic create --template <NAME>`). Every member lists and
//! reads them; only an admin creates, edits, removes and marks the default (the server refuses anyone else, exit
//! status 2). A template only prefills: editing or removing one changes no brief.
use super::{finish, text_arg};
use crate::client::Client;
use crate::relations::for_terminal;
use clap::Subcommand;
use reeve_protocol::*;

/// Clap's parser for `--level`.
fn parse_level(s: &str) -> Result<BriefTemplateLevel, String> {
    BriefTemplateLevel::from_wire(s).ok_or_else(|| {
        format!("`{s}` is neither initiative nor epic (a story has no brief template)")
    })
}

#[derive(Subcommand)]
pub enum Template {
    /// List brief templates: initiative templates first, then by name. The level's default is marked.
    Ls {
        /// Only this level's templates.
        #[arg(long, value_parser = parse_level, value_name = "initiative|epic")]
        level: Option<BriefTemplateLevel>,
    },
    /// Show one brief template with its body.
    Show {
        /// The template's id (`reeve template ls` prints it as #<ID>).
        id: i64,
    },
    /// Create a brief template (admins only). Idempotent by content (level, name, body): creating one that exists
    /// with the same body succeeds and changes nothing, so a directory of templates can be loaded again; the same
    /// name with another body is refused (use `edit`). It is not the default until `reeve template default <ID>`.
    Create {
        /// The level whose briefs it is for.
        #[arg(long, value_parser = parse_level, value_name = "initiative|epic")]
        level: BriefTemplateLevel,
        /// Its name, 1 to 100 characters on one line; unique at its level, whatever its case.
        #[arg(long)]
        name: String,
        /// The body, in Markdown.
        #[arg(long, required_unless_present = "body_file")]
        body: Option<String>,
        /// Read the body from this file, unchanged (no shell quoting in between); `-` reads stdin.
        #[arg(long, conflicts_with = "body", value_name = "PATH")]
        body_file: Option<std::path::PathBuf>,
    },
    /// Edit a brief template's name, body or both (admins only); the --version must be the one you last read.
    /// Briefs already made from it do not change.
    Edit {
        /// The template's id.
        id: i64,
        /// The template version you are editing (optimistic concurrency).
        #[arg(long)]
        version: i64,
        /// New name.
        #[arg(long)]
        name: Option<String>,
        /// New body.
        #[arg(long)]
        body: Option<String>,
        /// Read the new body from this file, unchanged; `-` reads stdin.
        #[arg(long, conflicts_with = "body", value_name = "PATH")]
        body_file: Option<std::path::PathBuf>,
    },
    /// Remove a brief template (admins only). Briefs already made from it do not change.
    Rm {
        /// The template's id.
        id: i64,
    },
    /// Make a template its level's default (admins only): the brief `reeve initiative create` and `reeve epic
    /// create` start from when they are given no brief, no --template and no --no-template, and what the web
    /// form shows first. It replaces the level's previous default. With --clear, take the mark off it instead.
    Default {
        /// The template's id.
        id: i64,
        /// Take the default mark off this template; its level then has no default.
        #[arg(long)]
        clear: bool,
    },
}

/// One template on one line: `#3 initiative Scenario (default)`. The name is stored text, so it prints through
/// `for_terminal`.
fn line(t: &BriefTemplateDto) -> String {
    format!(
        "#{} {} {}{}",
        t.id,
        t.level.wire(),
        for_terminal(&t.name),
        if t.is_default { " (default)" } else { "" }
    )
}

/// A template with its version and its body. The body keeps its line breaks, and each line prints through
/// `for_terminal`: every other control character (a tab, an escape sequence) is written as its escape, so stored
/// text cannot drive the reader's terminal. `--json` does not come through here and stays lossless.
fn shown(t: &BriefTemplateDto) -> String {
    let body: Vec<String> = t
        .body
        .lines()
        .map(|l| for_terminal(l).into_owned())
        .collect();
    format!("{} (version {})\n\n{}", line(t), t.version, body.join("\n"))
}

#[allow(clippy::ptr_arg)] // `finish` hands its renderer exactly `&Vec<_>`
fn listed(ts: &Vec<BriefTemplateDto>) -> String {
    if ts.is_empty() {
        "no brief templates".into()
    } else {
        ts.iter().map(line).collect::<Vec<_>>().join("\n")
    }
}

pub async fn run(c: &Client, cmd: Template, json: bool) -> anyhow::Result<i32> {
    Ok(match cmd {
        Template::Ls { level } => {
            let path = match level {
                Some(l) => format!("/brief-templates?level={}", l.wire()),
                None => "/brief-templates".to_string(),
            };
            finish(c.get::<Vec<BriefTemplateDto>>(&path).await?, json, listed)
        }
        Template::Show { id } => finish(
            c.get::<BriefTemplateDto>(&format!("/brief-templates/{id}"))
                .await?,
            json,
            shown,
        ),
        Template::Create {
            level,
            name,
            body,
            body_file,
        } => {
            // Clap requires one of the two, so there is a body; a blank one is the server's to refuse.
            let body = text_arg(body, body_file)?.unwrap_or_default();
            finish(
                c.post::<BriefTemplateDto>(
                    "/brief-templates",
                    serde_json::to_value(CreateBriefTemplate { level, name, body })?,
                )
                .await?,
                json,
                line,
            )
        }
        Template::Edit {
            id,
            version,
            name,
            body,
            body_file,
        } => finish(
            c.patch::<BriefTemplateDto>(
                &format!("/brief-templates/{id}"),
                serde_json::to_value(EditBriefTemplate {
                    version,
                    name,
                    body: text_arg(body, body_file)?,
                })?,
            )
            .await?,
            json,
            line,
        ),
        Template::Rm { id } => finish(
            c.delete::<serde_json::Value>(&format!("/brief-templates/{id}"))
                .await?,
            json,
            |_| format!("removed: brief template #{id}"),
        ),
        Template::Default { id, clear: true } => finish(
            c.delete::<BriefTemplateDto>(&format!("/brief-templates/{id}/default"))
                .await?,
            json,
            line,
        ),
        Template::Default { id, clear: false } => finish(
            c.post::<BriefTemplateDto>(
                &format!("/brief-templates/{id}/default"),
                serde_json::json!({}),
            )
            .await?,
            json,
            line,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(name: &str, body: &str, is_default: bool) -> BriefTemplateDto {
        BriefTemplateDto {
            id: 3,
            level: BriefTemplateLevel::Initiative,
            name: name.into(),
            body: body.into(),
            is_default,
            version: 2,
        }
    }

    #[test]
    fn a_template_prints_on_one_line_with_its_mark() {
        assert_eq!(
            line(&template("Scenario", "b", false)),
            "#3 initiative Scenario"
        );
        assert_eq!(
            line(&template("Scenario", "b", true)),
            "#3 initiative Scenario (default)"
        );
        assert_eq!(listed(&vec![]), "no brief templates");
        assert_eq!(
            listed(&vec![template("A", "b", true), template("B", "b", false)]),
            "#3 initiative A (default)\n#3 initiative B"
        );
    }

    #[test]
    fn stored_text_cannot_drive_the_terminal() {
        // A name stored before the server refused control characters, or by another server.
        assert_eq!(
            line(&template("Sce\u{1b}[2Jnario\nmore", "b", false)),
            "#3 initiative Sce\\u{1b}[2Jnario\\nmore"
        );
        // The body keeps its lines (a CRLF file's too); everything else that is a control character is escaped.
        let body = "# Title\r\n\r\n| a\t| b |\n\u{1b}]0;owned\u{7}\n";
        assert_eq!(
            shown(&template("Scenario", body, true)),
            "#3 initiative Scenario (default) (version 2)\n\n# Title\n\n| a\\t| b |\n\\u{1b}]0;owned\\u{7}"
        );
    }
}
