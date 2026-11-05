//! Atlassian Document Format to Markdown. Pure: every description and comment body goes through [`to_markdown`];
//! story descriptions go through [`split_criteria`], which also lifts the acceptance criteria out of the body.
//!
//! Jira text is literal: Markdown- and HTML-significant characters in it are escaped, so `<img src=x>` or `Vec<T>`
//! in a description stays text in Reeve and never becomes markup. Criteria are plain text (the client edits them as
//! text), so they are rendered without escaping.
//!
//! Links are checked and escaped at the source as well ([`link_destination`]): only absolute http, https and mailto
//! URLs link, and a hostile href cannot end its destination early. The web client's sanitiser is the second layer.

use reeve_protocol::CriterionDto;
use serde_json::{Value, json};

/// `issue_url` is `<site>/browse/<ISSUE-KEY>` of the issue the document belongs to (for a comment: its issue).
pub fn to_markdown(doc: &Value, issue_url: &str) -> String {
    Renderer::markdown(issue_url).blocks(children(doc), 0)
}

/// Stories only: the body without the lifted items, and the criteria. Lifts every `taskItem` anywhere (checked →
/// done) and the list items under the heading whose text equals `heading` (case-insensitive, a trailing colon
/// ignored) up to the next heading. The heading and its lists are found inside block containers too (`panel`,
/// `expand`, `layoutSection`/`layoutColumn`, `blockquote`, ...): a section opened inside a container ends with it,
/// a section open around a container reaches into it, and a heading inside a container ends the section around it.
/// A container left empty by the lifting is dropped.
pub fn split_criteria(doc: &Value, heading: &str, issue_url: &str) -> (String, Vec<CriterionDto>) {
    let mut split = Split {
        text: Renderer::plain(issue_url),
        wanted: heading_key(heading),
        criteria: Vec::new(),
    };
    let (kept, _) = split.blocks(children(doc), false);
    let body = Renderer::markdown(issue_url).blocks(&kept, 0);
    (body, split.criteria)
}

struct Split<'a> {
    text: Renderer<'a>,
    wanted: String,
    criteria: Vec<CriterionDto>,
}

impl Split<'_> {
    /// One run of sibling blocks: the blocks kept, and whether the criteria section is open after the last one.
    fn blocks(&mut self, nodes: &[Value], mut in_section: bool) -> (Vec<Value>, bool) {
        let mut kept = Vec::new();
        for node in nodes {
            if node_type(node) == "heading" {
                in_section = heading_key(&plain_text(node)) == self.wanted;
                kept.push(node.clone());
                continue;
            }
            if in_section && matches!(node_type(node), "bulletList" | "orderedList") {
                for item in children(node) {
                    let t = self.text.item_text(item);
                    if !t.is_empty() {
                        self.criteria.push(CriterionDto {
                            text: t,
                            done: false,
                        });
                    }
                }
                continue;
            }
            if is_block_container(node) {
                let (inner, open) = self.blocks(children(node), in_section);
                // a section opened inside the container ends with it; a heading inside it ends the one around it
                in_section = in_section && open;
                if !inner.is_empty() {
                    let mut n = node.clone();
                    n["content"] = json!(inner);
                    kept.push(n);
                }
                continue;
            }
            if let Some(n) = strip_tasks(node, &self.text, &mut self.criteria) {
                kept.push(n);
            }
        }
        (kept, in_section)
    }
}

/// A node holding blocks the criteria heading and its lists can sit among (`panel`, `expand`, `layoutSection`,
/// `layoutColumn`, `blockquote`, unknown containers): not a list or table (their items are not sections), not a
/// leaf, not a run of inline nodes.
fn is_block_container(node: &Value) -> bool {
    let c = children(node);
    !c.is_empty()
        && !c.iter().all(is_inline)
        && !matches!(
            node_type(node),
            "bulletList" | "orderedList" | "taskList" | "listItem" | "table" | "tableRow"
        )
}

/// Headings compare trimmed, case-insensitively, with a trailing colon ignored ("Acceptance criteria:").
fn heading_key(s: &str) -> String {
    s.trim().trim_end_matches(':').trim().to_lowercase()
}

/// Removes every `taskItem` under `node` (collected as criteria, in document order); a container left empty is
/// dropped. Everything else, including a node without `content`, is kept as it was.
fn strip_tasks(node: &Value, r: &Renderer, out: &mut Vec<CriterionDto>) -> Option<Value> {
    if node_type(node) == "taskItem" {
        let text = r.inline(children(node)).trim().to_string();
        if !text.is_empty() {
            let done = node["attrs"]["state"].as_str() == Some("DONE");
            out.push(CriterionDto { text, done });
        }
        return None;
    }
    // checked before `content`: a node with only attributes (`blockCard`, `rule`, `mediaSingle`...) holds no task
    // and is kept as it is
    if !contains_task(node) {
        return Some(node.clone());
    }
    let kept: Vec<Value> = children(node)
        .iter()
        .filter_map(|c| strip_tasks(c, r, out))
        .collect();
    if kept.is_empty() {
        return None;
    }
    let mut n = node.clone();
    n["content"] = json!(kept);
    Some(n)
}

fn contains_task(node: &Value) -> bool {
    node_type(node) == "taskItem" || children(node).iter().any(contains_task)
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

/// The inline node types of ADF; a container holding only these renders as one paragraph.
fn is_inline(node: &Value) -> bool {
    matches!(
        node_type(node),
        "text"
            | "hardBreak"
            | "mention"
            | "emoji"
            | "inlineCard"
            | "date"
            | "status"
            | "mediaInline"
            | "placeholder"
            | "inlineExtension"
    )
}

/// Every text node's text, concatenated: what an unknown leaf becomes.
fn plain_text(node: &Value) -> String {
    match node.get("text").and_then(Value::as_str) {
        Some(t) => t.to_string(),
        None => children(node).iter().map(plain_text).collect(),
    }
}

/// A `date` node keeps its value in `attrs.timestamp`: milliseconds since the epoch (a string in Jira's ADF, a number
/// accepted too), rendered as the UTC `YYYY-MM-DD`. An unparseable timestamp renders as nothing.
fn date_text(node: &Value) -> String {
    let ts = &node["attrs"]["timestamp"];
    let ms = ts
        .as_i64()
        .or_else(|| ts.as_str().and_then(|t| t.trim().parse::<i64>().ok()));
    ms.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// An extension (macro) node has no content of its own; its optional `attrs.text` is the readable fallback the
/// editor shows when the extension cannot render. Without it the macro renders as nothing (its `parameters` are
/// app-specific configuration, not text).
fn extension_text(node: &Value) -> String {
    node["attrs"]["text"].as_str().unwrap_or("").to_string()
}

/// Backslash-escapes the characters that would turn literal text into Markdown or HTML, and writes `&` as `&amp;` so
/// literal entity text (`&lt;`, `&copy;`) is not decoded into the character it names.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '&' {
            out.push_str("&amp;");
            continue;
        }
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escapes what would start a heading, list, rule or fence at the start of a line of paragraph text.
fn escape_line_starts(s: &str) -> String {
    s.split('\n')
        .map(|line| {
            let body = line.trim_start_matches(' ');
            let lead = &line[..line.len() - body.len()];
            if body.starts_with(['#', '-', '+', '=', '~']) {
                return format!("{lead}\\{body}");
            }
            let digits = body.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && body[digits..].starts_with(['.', ')']) {
                return format!("{lead}{}\\{}", &body[..digits], &body[digits..]);
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The Markdown link destination for `url`, or `None` when it must not be a link. Only an absolute `http://`,
/// `https://` or `mailto:` URL (case-insensitive, surrounding whitespace trimmed) links: a script or data scheme never
/// does, and neither does a relative one, which would resolve against Reeve's origin rather than the Jira site. The
/// destination cannot end early or be decoded into something else: `\`, `(` and `)` are backslash-escaped, `&` is
/// written `&amp;` (as in text), and whitespace, controls, `"`, `<`, `>` and the backtick are percent-encoded.
fn link_destination(url: &str) -> Option<String> {
    let url = url.trim();
    let allowed = ["http://", "https://", "mailto:"].iter().any(|p| {
        url.len() > p.len() && url.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())
    });
    if !allowed {
        return None;
    }
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\\' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '`') => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    Some(out)
}

/// The longest run of backticks in `s`.
fn longest_backtick_run(s: &str) -> usize {
    s.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// A code span whose delimiter is longer than any backtick run inside it.
fn code_span(s: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(s) + 1);
    let pad = if s.starts_with('`') || s.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{s}{pad}{fence}")
}

struct Renderer<'a> {
    issue_url: &'a str,
    /// Markdown output escapes literal text; plain output (criteria) does not.
    markdown: bool,
}

impl<'a> Renderer<'a> {
    fn markdown(issue_url: &'a str) -> Self {
        Self {
            issue_url,
            markdown: true,
        }
    }
    fn plain(issue_url: &'a str) -> Self {
        Self {
            issue_url,
            markdown: false,
        }
    }

    fn esc(&self, s: &str) -> String {
        if self.markdown {
            escape(s)
        } else {
            s.to_string()
        }
    }

    /// Blocks separated by a blank line, each indented by `indent` spaces after its first line.
    fn blocks(&self, nodes: &[Value], indent: usize) -> String {
        nodes
            .iter()
            .map(|n| self.block(n, indent))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn paragraph(&self, nodes: &[Value]) -> String {
        let s = self.inline(nodes);
        if self.markdown {
            escape_line_starts(&s)
        } else {
            s
        }
    }

    fn block(&self, node: &Value, indent: usize) -> String {
        let out = match node_type(node) {
            "paragraph" => self.paragraph(children(node)),
            "heading" => {
                let level = node["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as usize;
                format!("{} {}", "#".repeat(level), self.inline(children(node)))
            }
            "bulletList" | "orderedList" | "taskList" => self.list(node, indent),
            "codeBlock" => {
                let lang = node["attrs"]["language"].as_str().unwrap_or("");
                let code = plain_text(node);
                let fence = "`".repeat((longest_backtick_run(&code) + 1).max(3));
                format!("{fence}{lang}\n{code}\n{fence}")
            }
            "blockquote" => self
                .blocks(children(node), 0)
                .lines()
                .map(|l| format!("> {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
            "rule" => "---".into(),
            "blockCard" | "embedCard" => self.card(node),
            "media" => self.media(),
            "mediaSingle" | "mediaGroup" => children(node)
                .iter()
                .map(|c| self.block(c, indent))
                .collect::<Vec<_>>()
                .join(" "),
            "table" => self.table(node),
            "extension" => self.esc(&extension_text(node)),
            // Unknown containers (panel, expand, layout, decision lists, ...): their children, as blocks or as a
            // paragraph; an unknown leaf: its text.
            _ if children(node).is_empty() => self.esc(&plain_text(node)),
            _ if children(node).iter().all(is_inline) => self.paragraph(children(node)),
            _ => self.blocks(children(node), indent),
        };
        out.trim_end().to_string()
    }

    /// One row per line, cells joined by ` | `.
    fn table(&self, node: &Value) -> String {
        children(node)
            .iter()
            .map(|row| {
                children(row)
                    .iter()
                    .map(|cell| {
                        self.blocks(children(cell), 0)
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .filter(|r| !r.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// One line per item; a nested list is indented two spaces under its item.
    fn list(&self, node: &Value, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut lines = Vec::new();
        for item in children(node) {
            match node_type(item) {
                "listItem" => {
                    let marker = if node_type(node) == "orderedList" {
                        "1."
                    } else {
                        "-"
                    };
                    let mut first = true;
                    for c in children(item) {
                        if matches!(node_type(c), "bulletList" | "orderedList" | "taskList") {
                            lines.push(self.list(c, indent + 2));
                        } else if first {
                            lines.push(format!("{pad}{marker} {}", self.block(c, indent + 2)));
                            first = false;
                        } else {
                            lines.push(format!("{pad}  {}", self.block(c, indent + 2)));
                        }
                    }
                    if first && children(item).is_empty() {
                        lines.push(format!("{pad}{marker}"));
                    }
                }
                "taskItem" => {
                    let mark = if item["attrs"]["state"].as_str() == Some("DONE") {
                        "x"
                    } else {
                        " "
                    };
                    lines.push(format!("{pad}- [{mark}] {}", self.inline(children(item))));
                }
                // A taskList nests taskLists directly.
                "taskList" | "bulletList" | "orderedList" => {
                    lines.push(self.list(item, indent + 2));
                }
                _ => lines.push(format!("{pad}{}", self.block(item, indent + 2))),
            }
        }
        lines.join("\n")
    }

    /// A list item as one criterion line.
    fn item_text(&self, item: &Value) -> String {
        children(item)
            .iter()
            .map(|c| self.block(c, 0))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }

    fn inline(&self, nodes: &[Value]) -> String {
        nodes.iter().map(|n| self.inline_node(n)).collect()
    }

    fn inline_node(&self, node: &Value) -> String {
        match node_type(node) {
            "text" => self.marked(node),
            "hardBreak" => "  \n".into(),
            "mention" => {
                let text = node["attrs"]["text"].as_str().unwrap_or("");
                format!("@{}", self.esc(text.trim_start_matches('@')))
            }
            "emoji" => self.esc(
                node["attrs"]["text"]
                    .as_str()
                    .or_else(|| node["attrs"]["shortName"].as_str())
                    .unwrap_or(""),
            ),
            "inlineCard" | "blockCard" | "embedCard" => self.card(node),
            "media" | "mediaInline" => self.media(),
            "date" => self.esc(&date_text(node)),
            "status" => {
                let text = node["attrs"]["text"].as_str().unwrap_or("").trim();
                if text.is_empty() {
                    String::new()
                } else {
                    self.esc(&format!("[{text}]"))
                }
            }
            "inlineExtension" => self.esc(&extension_text(node)),
            _ => self.esc(&plain_text(node)),
        }
    }

    /// A media node's `attrs.id` is a Media Services id, not a URL: link to the issue page, which lists its
    /// attachments.
    fn media(&self) -> String {
        if !self.markdown {
            return format!("[attachment]({})", self.issue_url);
        }
        match link_destination(self.issue_url) {
            Some(dest) => format!("[attachment]({dest})"),
            None => "attachment".into(),
        }
    }

    /// A card links its URL to itself; one that may not link (see [`link_destination`]) is its URL as text.
    fn card(&self, node: &Value) -> String {
        let url = node["attrs"]["url"].as_str().unwrap_or("");
        if !self.markdown {
            return format!("[{url}]({url})");
        }
        match link_destination(url) {
            Some(dest) => format!("[{}]({dest})", self.esc(url)),
            None => self.esc(url),
        }
    }

    /// Text with its marks: code innermost (never escaped), then emphasis, strong, and a link outermost.
    fn marked(&self, node: &Value) -> String {
        let raw = node["text"].as_str().unwrap_or("");
        let marks: Vec<&Value> = node
            .get("marks")
            .and_then(Value::as_array)
            .map(|m| m.iter().collect())
            .unwrap_or_default();
        let has = |t: &str| marks.iter().any(|m| node_type(m) == t);
        let mut s = if has("code") && self.markdown {
            code_span(raw)
        } else {
            self.esc(raw)
        };
        if !self.markdown {
            return s;
        }
        if has("em") {
            s = format!("_{s}_");
        }
        if has("strong") {
            s = format!("**{s}**");
        }
        if let Some(link) = marks.iter().find(|m| node_type(m) == "link") {
            // A link that may not link keeps its text (see [`link_destination`]).
            if let Some(dest) = link_destination(link["attrs"]["href"].as_str().unwrap_or("")) {
                s = format!("[{s}]({dest})");
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Value> {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/jira/adf_samples.json"
        ))
        .unwrap();
        serde_json::from_str::<Value>(&text)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    }

    fn sample(name: &str) -> Value {
        samples()
            .into_iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("no sample {name}"))
    }

    #[test]
    fn every_sample_renders_as_expected() {
        let all = samples();
        assert_eq!(all.len(), 17);
        for s in all {
            let name = s["name"].as_str().unwrap();
            let url = s["issue_url"].as_str().unwrap();
            let expected_md = s["expected_markdown"].as_str().unwrap();
            match s.get("expected_criteria") {
                Some(c) => {
                    let expected: Vec<CriterionDto> = serde_json::from_value(c.clone()).unwrap();
                    let (body, criteria) = split_criteria(&s["adf"], "acceptance criteria", url);
                    assert_eq!(body, expected_md, "body of {name}");
                    assert_eq!(criteria, expected, "criteria of {name}");
                }
                None => assert_eq!(to_markdown(&s["adf"], url), expected_md, "{name}"),
            }
        }
    }

    #[test]
    fn media_links_to_the_issue_page_not_its_media_id() {
        let s = sample("media in mediaSingle");
        let md = to_markdown(&s["adf"], "https://acme.atlassian.net/browse/SCRUM-11");
        assert_eq!(
            md,
            "[attachment](https://acme.atlassian.net/browse/SCRUM-11)"
        );
        assert!(!md.contains("6e7c7f2c"));
    }

    #[test]
    fn to_markdown_never_lifts_criteria() {
        let s = sample("criteria section with mixed done items");
        let md = to_markdown(&s["adf"], "u");
        assert!(md.contains("- Totals add up"));
        assert!(md.contains("- [x] Reviewed by finance"));
        let (body, _) = split_criteria(&s["adf"], "Acceptance criteria", "u");
        assert!(
            !body.contains("Totals add up"),
            "lifted items stay out of the body"
        );
        assert!(
            !body.contains("Reviewed by"),
            "lifted items stay out of the body"
        );
        assert!(body.contains("Currency conversion"), "other lists stay");
    }

    #[test]
    fn a_document_without_the_heading_lifts_only_task_items() {
        let doc = json!({"type": "doc", "content": [
            {"type": "bulletList", "content": [{"type": "listItem", "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "keep me"}]}]}]}
        ]});
        let (body, criteria) = split_criteria(&doc, "Acceptance criteria", "u");
        assert_eq!(body, "- keep me");
        assert!(criteria.is_empty());
    }

    fn para(text: &str, marks: Value) -> Value {
        json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": text, "marks": marks}]}]})
    }

    /// Catches Jira text reinterpreted as HTML (a live `<img>`) or swallowed as an unknown tag.
    #[test]
    fn html_in_jira_text_stays_literal() {
        let md = to_markdown(&para("<img src=x> Vec<T>", json!([])), "u");
        assert_eq!(md, "\\<img src=x\\> Vec\\<T\\>");
        assert!(
            !md.replace("\\<", "").contains('<'),
            "no unescaped angle bracket"
        );
    }

    /// Catches literal entity text decoded by the Markdown renderer: `&lt;` must show as `&lt;`, not `<`.
    #[test]
    fn entity_text_in_jira_text_stays_literal() {
        let md = to_markdown(&para("a &lt;b&gt; & c", json!([])), "u");
        assert_eq!(md, "a &amp;lt;b&amp;gt; &amp; c");
        let code = to_markdown(&para("&lt;", json!([{"type": "code"}])), "u");
        assert_eq!(code, "`&lt;`", "a code span is literal already");
        let (_, criteria) = split_criteria(
            &json!({"type": "doc", "content": [{"type": "taskList", "content": [{"type": "taskItem",
                "attrs": {"state": "TODO"}, "content": [{"type": "text", "text": "a & b"}]}]}]}),
            "acceptance criteria",
            "u",
        );
        assert_eq!(criteria[0].text, "a & b", "criteria are plain text");
    }

    fn linked(href: &str) -> String {
        to_markdown(
            &para("click", json!([{"type": "link", "attrs": {"href": href}}])),
            "https://site.example/browse/K-1",
        )
    }

    fn card_doc(url: &str) -> Value {
        json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "inlineCard", "attrs": {"url": url}}]}]})
    }

    /// Catches a script, data or relative href written into a link: only absolute http, https and mailto URLs link;
    /// anything else keeps its text, unlinked.
    #[test]
    fn a_link_with_an_unsafe_scheme_keeps_only_its_text() {
        for href in [
            "javascript:alert(1)",
            "  JavaScript:alert(1)",
            "\tjava\tscript:alert(1)",
            "\u{1}javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "/relative/path",
            "//evil.example/x",
            "#fragment",
            "https",
            "",
        ] {
            assert_eq!(linked(href), "click", "{href:?}");
        }
        assert_eq!(
            linked(" HTTPS://a.example/x "),
            "[click](HTTPS://a.example/x)"
        );
        assert_eq!(
            linked("mailto:ana@example.com"),
            "[click](mailto:ana@example.com)"
        );
    }

    /// Catches an href that ends the Markdown destination early and injects markup or a second link after it.
    #[test]
    fn a_link_href_cannot_break_out_of_its_destination() {
        assert_eq!(
            linked("https://a.example/x) <img src=x onerror=alert(1)>"),
            "[click](https://a.example/x\\)%20%3Cimg%20src=x%20onerror=alert\\(1\\)%3E)"
        );
        assert_eq!(
            linked("https://a.example/\"onmouseover=\"alert(1)"),
            "[click](https://a.example/%22onmouseover=%22alert\\(1\\))"
        );
        assert_eq!(
            linked("https://a.example/a\n\n<b>x</b>\\"),
            "[click](https://a.example/a%0A%0A%3Cb%3Ex%3C/b%3E\\\\)"
        );
        assert_eq!(
            linked("https://a.example/a\u{a0}b`c"),
            "[click](https://a.example/a%C2%A0b%60c)"
        );
        // Literal entity text in a URL stays literal, as it does in text.
        assert_eq!(
            linked("https://a.example/?a=1&amp;b=2"),
            "[click](https://a.example/?a=1&amp;amp;b=2)"
        );
    }

    /// Catches a card or media link that bypasses the destination rules.
    #[test]
    fn cards_and_media_follow_the_same_link_rules() {
        assert_eq!(
            to_markdown(&card_doc("javascript:alert(1)"), "u"),
            "javascript:alert(1)"
        );
        assert_eq!(
            to_markdown(&card_doc("https://a.example/x) <b>"), "u"),
            "[https://a.example/x) \\<b\\>](https://a.example/x\\)%20%3Cb%3E)"
        );
        let media = json!({"type": "doc", "content": [{"type": "mediaSingle", "content": [
            {"type": "media", "attrs": {"id": "m1", "type": "file"}}]}]});
        assert_eq!(to_markdown(&media, "javascript:alert(1)"), "attachment");
        let (_, criteria) = split_criteria(
            &json!({"type": "doc", "content": [{"type": "taskList", "content": [{"type": "taskItem",
                "attrs": {"state": "TODO"}, "content": [{"type": "inlineCard", "attrs": {"url": "https://a.example/c"}}]}]}]}),
            "acceptance criteria",
            "u",
        );
        assert_eq!(
            criteria[0].text, "[https://a.example/c](https://a.example/c)",
            "criteria are plain text: unchanged"
        );
    }

    /// Catches a code span that a backtick inside it would close early.
    #[test]
    fn a_backtick_inside_a_code_span_is_fenced_by_a_longer_run() {
        let md = to_markdown(&para("a`b", json!([{"type": "code"}])), "u");
        assert_eq!(md, "``a`b``");
        let md = to_markdown(&para("``x", json!([{"type": "code"}])), "u");
        assert_eq!(md, "``` ``x ```");
    }

    /// Catches attribute-backed inline nodes (date, status) falling through to the text fallback and vanishing from a
    /// criterion, and a criterion that escapes the status brackets.
    #[test]
    fn dates_and_statuses_render_in_criteria_as_plain_text() {
        let doc = json!({"type": "doc", "content": [{"type": "taskList", "content": [{"type": "taskItem",
            "attrs": {"state": "TODO"}, "content": [
                {"type": "text", "text": "Ship by "},
                {"type": "date", "attrs": {"timestamp": "1793059200000"}},
                {"type": "text", "text": " "},
                {"type": "status", "attrs": {"text": "IN REVIEW", "color": "blue"}}]}]}]});
        let (_, criteria) = split_criteria(&doc, "Acceptance criteria", "u");
        assert_eq!(criteria.len(), 1);
        assert_eq!(criteria[0].text, "Ship by 2026-10-27 [IN REVIEW]");
        // An unparseable timestamp renders as nothing rather than the raw attribute.
        let bad = json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "on "}, {"type": "date", "attrs": {"timestamp": "soon"}}]}]});
        assert_eq!(to_markdown(&bad, "u"), "on");
    }

    /// Catches a panel whose paragraphs are glued together.
    #[test]
    fn unknown_containers_keep_their_block_breaks() {
        let s = sample("unknown node");
        assert!(to_markdown(&s["adf"], "u").starts_with("Heads up.\n\nSecond line."));
    }

    #[test]
    fn criteria_are_plain_text_not_escaped_markdown() {
        let s = sample("a criteria heading with a trailing colon");
        let (_, criteria) = split_criteria(&s["adf"], "Acceptance criteria", "u");
        assert_eq!(criteria[0].text, "Uses <Vec> as is");
    }

    /// Catches an attribute-only node (no `content`) dropped by the task-stripping pass.
    #[test]
    fn a_node_without_content_survives_split_criteria() {
        let doc = json!({"type": "doc", "content": [
            {"type": "blockCard", "attrs": {"url": "https://example.com/x"}}
        ]});
        let (body, criteria) = split_criteria(&doc, "Acceptance criteria", "u");
        assert_eq!(body, "[https://example.com/x](https://example.com/x)");
        assert!(criteria.is_empty());
    }

    /// Catches `split_criteria` changing a document that holds no criteria at all.
    #[test]
    fn a_document_without_tasks_renders_as_to_markdown_does() {
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "one"}]},
            {"type": "rule"},
            {"type": "embedCard", "attrs": {"url": "https://example.com/e"}},
            {"type": "mediaSingle", "content": [{"type": "media", "attrs": {"id": "m1", "type": "file"}}]},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "a"}, {"type": "hardBreak"}, {"type": "text", "text": "b"},
                {"type": "inlineCard", "attrs": {"url": "https://example.com/i"}}]}
        ]});
        let (body, criteria) = split_criteria(&doc, "Acceptance criteria", "https://j/browse/A-1");
        assert_eq!(body, to_markdown(&doc, "https://j/browse/A-1"));
        assert!(body.contains("https://example.com/e") && body.contains("https://example.com/i"));
        assert!(criteria.is_empty());
        // every sample with neither a task nor the criteria heading passes through unchanged
        for s in samples() {
            let adf = &s["adf"];
            if !contains_task(adf)
                && !adf
                    .to_string()
                    .to_lowercase()
                    .contains("acceptance criteria")
            {
                let (b, _) = split_criteria(adf, "acceptance criteria", "u");
                assert_eq!(b, to_markdown(adf, "u"), "{}", s["name"]);
            }
        }
    }

    fn p(text: &str) -> Value {
        json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
    }

    fn h(text: &str) -> Value {
        json!({"type": "heading", "attrs": {"level": 3}, "content": [{"type": "text", "text": text}]})
    }

    fn bullets(items: &[&str]) -> Value {
        let items: Vec<Value> = items
            .iter()
            .map(|t| json!({"type": "listItem", "content": [p(t)]}))
            .collect();
        json!({"type": "bulletList", "content": items})
    }

    /// Catches criteria inside a panel left in the description and never lifted.
    #[test]
    fn criteria_inside_a_panel_are_lifted() {
        let doc = json!({"type": "doc", "content": [
            p("Intro."),
            {"type": "panel", "attrs": {"panelType": "info"}, "content": [
                h("Acceptance criteria"), bullets(&["Totals add up", "Export works"])]},
            bullets(&["Not a criterion"])
        ]});
        let (body, criteria) = split_criteria(&doc, "Acceptance criteria", "u");
        let texts: Vec<&str> = criteria.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Totals add up", "Export works"]);
        assert!(!body.contains("Totals add up") && !body.contains("Export works"));
        assert!(
            body.contains("- Not a criterion"),
            "the section ends with the panel"
        );
        assert_eq!(
            body,
            "Intro.\n\n### Acceptance criteria\n\n- Not a criterion"
        );
    }

    /// Catches a heading inside an expand (inside a layout) not opening the section, and an emptied container kept.
    #[test]
    fn a_heading_inside_an_expand_lifts_its_items_and_drops_emptied_containers() {
        let doc = json!({"type": "doc", "content": [
            {"type": "layoutSection", "content": [{"type": "layoutColumn", "content": [
                {"type": "expand", "attrs": {"title": "Details"}, "content": [
                    h("Acceptance criteria:"),
                    {"type": "orderedList", "content": [
                        {"type": "listItem", "content": [p("First")]},
                        {"type": "listItem", "content": [p("Second")]}]},
                    h("Notes"),
                    bullets(&["stays"])]}]}]},
            {"type": "blockquote", "content": [{"type": "taskList", "content": [
                {"type": "taskItem", "attrs": {"state": "DONE"}, "content": [{"type": "text", "text": "Quoted task"}]}]}]}
        ]});
        let (body, criteria) = split_criteria(&doc, "Acceptance criteria", "u");
        let got: Vec<(&str, bool)> = criteria.iter().map(|c| (c.text.as_str(), c.done)).collect();
        assert_eq!(
            got,
            [("First", false), ("Second", false), ("Quoted task", true)]
        );
        assert!(!body.contains("First") && !body.contains("Quoted task"));
        assert!(body.contains("- stays"), "a later heading ends the section");
        assert!(!body.contains('>'), "the emptied blockquote is dropped");
    }

    /// Catches a heading opened around a container not reaching the list inside it, and a nested heading not
    /// closing the section around it.
    #[test]
    fn a_section_reaches_into_a_container_and_a_nested_heading_ends_it() {
        let doc = json!({"type": "doc", "content": [
            h("Acceptance criteria"),
            {"type": "panel", "content": [bullets(&["Inside"]), h("Other")]},
            bullets(&["After"])
        ]});
        let (body, criteria) = split_criteria(&doc, "Acceptance criteria", "u");
        let texts: Vec<&str> = criteria.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Inside"]);
        assert!(body.contains("- After"));
    }

    #[test]
    fn a_missing_or_null_document_is_empty() {
        assert_eq!(to_markdown(&Value::Null, "u"), "");
        assert_eq!(
            split_criteria(&Value::Null, "x", "u"),
            (String::new(), vec![])
        );
    }
}
