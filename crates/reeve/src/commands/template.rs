//! `reeve template`: brief templates, the named Markdown texts a new initiative's or epic's brief can start from
//! (`reeve initiative create --template <NAME>`, `reeve epic create --template <NAME>`). Every member lists and
//! reads them; only an admin creates, edits, removes and marks the default (the server refuses anyone else, exit
//! status 2). A template only prefills: editing or removing one changes no brief.
use super::{finish, text_arg};
use crate::client::Client;
use crate::terminal;
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
    /// List brief templates: initiative templates first, then by name. Each level's default is marked `(default)`.
    Ls {
        /// Only this level's templates.
        #[arg(long, value_parser = parse_level, value_name = "initiative|epic")]
        level: Option<BriefTemplateLevel>,
    },
    /// Show one brief template: its line, its version (what `edit --version` takes) and its body.
    Show {
        /// The template's id (`reeve template ls` prints it as #<ID>).
        id: i64,
    },
    /// Create a brief template (admins only). Idempotent by content (level, name, body): creating one that exists
    /// with the same body succeeds and changes nothing, so a directory of templates can be loaded again; the same
    /// name with another body is refused (use `edit`). It is not the default until `reeve template default <ID>`.
    /// One of --body and --body-file is required.
    Create {
        /// The level whose briefs it is for.
        #[arg(long, value_parser = parse_level, value_name = "initiative|epic")]
        level: BriefTemplateLevel,
        /// Its name, 1 to 100 characters on one line; unique at its level, whatever its case.
        #[arg(long)]
        name: String,
        /// The body, in Markdown. One of --body and --body-file is required.
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
        /// The version you last read (`reeve template show <ID>` prints it). The edit is refused with a conflict
        /// if the template has changed since.
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
    /// create` start from when they are given no brief, no --template and no --no-template. It replaces the
    /// level's previous default. With --clear, take the mark off it instead.
    Default {
        /// The template's id.
        id: i64,
        /// Take the default mark off this template; its level then has no default.
        #[arg(long)]
        clear: bool,
    },
}

/// One template on one line: `#3 initiative Scenario (default)`. The name is stored text, so it prints through
/// `terminal::line`.
fn line(t: &BriefTemplateDto) -> String {
    format!(
        "#{} {} {}{}",
        t.id,
        t.level.wire(),
        terminal::line(&t.name),
        if t.is_default { " (default)" } else { "" }
    )
}

/// What `edit` prints: the template's line and the version it has now, so a second edit needs no `show` in between.
fn edited(t: &BriefTemplateDto) -> String {
    format!("{} (version {})", line(t), t.version)
}

/// A template with its version and its body. The body is long text and prints through `terminal::block`: it keeps
/// its line breaks and tabs (a CRLF file's lines are lines), and everything that can drive the reader's terminal is
/// written as its escape. `--json` does not come through here and is the exact, round-trip form.
fn shown(t: &BriefTemplateDto) -> String {
    let body = terminal::block(&t.body);
    // One trailing line break is the file's own ending, not an empty last line to print.
    let body = body.strip_suffix('\n').unwrap_or(&body);
    format!("{}\n\n{body}", edited(t))
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
            edited,
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
    fn an_edit_prints_the_version_the_next_edit_takes() {
        assert_eq!(
            edited(&template("Scenario", "b", true)),
            "#3 initiative Scenario (default) (version 2)"
        );
        assert_eq!(
            edited(&template("Scenario", "b", false)),
            "#3 initiative Scenario (version 2)"
        );
    }

    #[test]
    fn stored_text_cannot_drive_the_terminal() {
        // A name stored before the server refused control characters, or by another server.
        assert_eq!(
            line(&template("Sce\u{1b}[2Jnario\nmore", "b", false)),
            "#3 initiative Sce\\u{1b}[2Jnario\\nmore"
        );
        // The body keeps its lines (a CRLF file's too) and its tabs; everything else that is a control character is
        // escaped.
        let body = "# Title\r\n\r\n| a\t| b |\n\u{1b}]0;owned\u{7}\n";
        assert_eq!(
            shown(&template("Scenario", body, true)),
            "#3 initiative Scenario (default) (version 2)\n\n# Title\n\n| a\t| b |\n\\u{1b}]0;owned\\u{7}"
        );
    }

    /// A template's name is one line in every form it is printed in (its line, the list, the edit answer, `show`);
    /// its body is long text and keeps its line breaks. Only `show` prints the body, so only its call names it long.
    #[test]
    fn a_templates_name_stays_on_its_line_and_its_body_keeps_its_lines() {
        use crate::hostile::{Paths, assert_one_line, assert_shown};
        let t = template("Scenario", "Long text", true);
        assert_eq!(assert_one_line(&t, &Paths::none(), line), ["/name"]);
        assert_eq!(assert_one_line(&t, &Paths::none(), edited), ["/name"]);
        assert_eq!(
            assert_one_line(&t, &Paths::none().long(&["/body"]), shown),
            ["/name"]
        );
        let ts = vec![t.clone(), template("Other", "Long text", false)];
        assert_shown(
            &assert_one_line(&ts, &Paths::none(), listed),
            &["/0/name", "/1/name"],
        );
        // The hostile helper's long text holds no line break; a body forced through `line` shows here as `\n`.
        let two = template("Scenario", "first line\nsecond line\n", false);
        assert!(
            shown(&two).ends_with("\n\nfirst line\nsecond line"),
            "{}",
            shown(&two)
        );
    }
}
