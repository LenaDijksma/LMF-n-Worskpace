//! A UI-independent *view model* of a document: a flat list of [`Card`]s.
//!
//! Front ends (the Slint desktop app today, anything else later) map cards to widgets, so the
//! logic of "what does this document look like" is written and tested once, without any GUI.
//!
//! This is a block-level approximation, not a Markdown renderer: headings, paragraphs and code
//! fences are recognized; inline Markdown (`**bold**`, links, …) is shown as typed until the
//! Markdown baseline engine arrives (ADR-001). Unknown extensions are shown as their source.

use crate::frontmatter::MetaValue;
use crate::lines::{fence_close, fence_open};
use crate::model::{Block, Directive, Document};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardKind {
    /// Document metadata summary (from front matter).
    Meta,
    /// Heading level 1–6; the text is in `body`.
    Heading(u8),
    Paragraph,
    /// Fenced code; the info string (language) is in `title`.
    Code,
    /// `lmf:callout`; the payload is its `kind` (e.g. `"warning"`).
    Callout(String),
    Details,
    TabGroup,
    Tab,
    /// A block quote; `body` is the quoted Markdown with the `>` markers removed.
    Quote,
    /// A thematic break (`---`).
    Rule,
    /// A GitHub-style table.
    Table(TableData),
    /// `lmf:math`; `body` is the LaTeX source (not rendered).
    Math,
    /// `lmf:diagram`; `body` is the diagram source (never executed or rendered here).
    Diagram,
    /// `lmf:include` (transclusion). `body` is the author-written fallback text. A front end
    /// that knows the workspace replaces this card with the included content.
    Include {
        document: String,
        section: Option<String>,
        block: Option<String>,
    },
    /// An extension this implementation does not understand; `body` is its source.
    Unknown,
}

/// Column alignment of a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// A parsed table. All rows have exactly as many cells as the header; cell text is raw inline
/// Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableData {
    pub header: Vec<String>,
    pub align: Vec<Align>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub kind: CardKind,
    pub title: String,
    pub body: String,
    /// Nesting depth (0 = top level), for indentation.
    pub depth: usize,
}

/// Cards for a piece of LMF/Markdown text (e.g. included content), starting at `depth`.
/// Front matter is ignored.
pub fn cards_from_text(text: &str, depth: usize) -> Vec<Card> {
    let doc = crate::parser::parse(text);
    let mut out = Vec::new();
    walk_blocks(&doc, &doc.blocks, depth, &mut out);
    out
}

/// Builds the card list for a parsed document.
pub fn build_cards(doc: &Document) -> Vec<Card> {
    let mut out = Vec::new();

    if let Some(fm) = &doc.front_matter {
        let title = match fm.get("title").map(|e| &e.value) {
            Some(MetaValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let body = fm
            .entries
            .iter()
            .filter(|e| e.key != "title" && e.key != "lmf_version")
            .map(|e| format!("{}: {}", e.key, plain(&e.value)))
            .collect::<Vec<String>>()
            .join("\n");
        if !title.is_empty() || !body.is_empty() {
            out.push(Card {
                kind: CardKind::Meta,
                title,
                body,
                depth: 0,
            });
        }
    }

    walk_blocks(doc, &doc.blocks, 0, &mut out);
    out
}

fn walk_blocks(doc: &Document, blocks: &[Block], depth: usize, out: &mut Vec<Card>) {
    for block in blocks {
        match block {
            Block::Markdown(m) => markdown_cards(doc.text(m.span), depth, out),
            Block::Directive(d) => walk_directive(doc, d, depth, out),
        }
    }
}

fn walk_directive(doc: &Document, dir: &Directive, depth: usize, out: &mut Vec<Card>) {
    let (kind, title, merge_body) = match (dir.namespace.as_str(), dir.name.as_str()) {
        ("lmf", "callout") => {
            let k = dir.attr_str("kind").unwrap_or("note").to_string();
            let label = capitalize(&k);
            let title = match dir.attr_str("title") {
                Some(t) if !t.is_empty() => format!("{} \u{2014} {}", label, t),
                _ => label,
            };
            (CardKind::Callout(k), title, true)
        }
        ("lmf", "details") => {
            let title = match dir.attr_str("title") {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => "Details".to_string(),
            };
            (CardKind::Details, title, true)
        }
        ("lmf", "tabs") => (CardKind::TabGroup, "Tabs".to_string(), false),
        ("lmf", "tab") => {
            let title = match dir.attr_str("title") {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => "Tab".to_string(),
            };
            (CardKind::Tab, title, true)
        }
        ("lmf", "math") => (
            CardKind::Math,
            "Math (LaTeX) \u{2014} shown as source".to_string(),
            true,
        ),
        ("lmf", "diagram") => {
            let kind = dir.attr_str("type").unwrap_or("unknown");
            (
                CardKind::Diagram,
                format!("Diagram ({}) \u{2014} shown as source", kind),
                true,
            )
        }
        ("lmf", "include") => {
            let document = dir.attr_str("document").unwrap_or("").to_string();
            let section = dir.attr_str("section").map(|s| s.to_string());
            let block = dir.attr_str("block").map(|s| s.to_string());
            let mut title = format!("Include: {}", document);
            if let Some(s) = &section {
                title.push_str(&format!(" \u{00a7} {}", s));
            }
            if let Some(b) = &block {
                title.push_str(&format!(" \u{00a7} block {}", b));
            }
            (
                CardKind::Include {
                    document,
                    section,
                    block,
                },
                title,
                true,
            )
        }
        _ => {
            out.push(Card {
                kind: CardKind::Unknown,
                title: format!(
                    "{} (unsupported extension, shown as source)",
                    dir.qualified_name()
                ),
                body: doc.text(dir.span).replace("\r\n", "\n"),
                depth,
            });
            return;
        }
    };

    // Leading Markdown children become the card's own body; the rest are nested cards.
    let mut body_parts: Vec<String> = Vec::new();
    let mut rest_from = 0usize;
    if merge_body {
        for (i, child) in dir.body.iter().enumerate() {
            match child {
                Block::Markdown(m) => {
                    body_parts.push(doc.text(m.span).replace("\r\n", "\n"));
                    rest_from = i + 1;
                }
                Block::Directive(_) => break,
            }
        }
    }
    out.push(Card {
        kind,
        title,
        body: body_parts.join("\n\n"),
        depth,
    });
    walk_blocks(doc, &dir.body[rest_from..], depth + 1, out);
}

fn markdown_cards(text: &str, depth: usize, out: &mut Vec<Card>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut para: Vec<&str> = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let raw = lines[i];

        // Fenced code.
        if let Some((ch, len)) = fence_open(raw) {
            flush_paragraph(&mut para, depth, out);
            let info = raw
                .trim()
                .trim_start_matches(|c: char| c == '`' || c == '~')
                .trim()
                .to_string();
            let mut code: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() {
                if fence_close(lines[i], ch, len) {
                    i += 1;
                    break;
                }
                code.push(lines[i]);
                i += 1;
            }
            out.push(Card {
                kind: CardKind::Code,
                title: info,
                body: code.join("\n"),
                depth,
            });
            continue;
        }

        // Setext heading: a paragraph underlined with === or ---.
        if !para.is_empty() {
            if let Some(level) = setext_level(raw) {
                let text = para
                    .iter()
                    .map(|l| l.trim())
                    .collect::<Vec<&str>>()
                    .join(" ");
                para.clear();
                out.push(Card {
                    kind: CardKind::Heading(level),
                    title: String::new(),
                    body: text,
                    depth,
                });
                i += 1;
                continue;
            }
        }

        // ATX heading.
        if let Some((level, t)) = heading(raw) {
            flush_paragraph(&mut para, depth, out);
            out.push(Card {
                kind: CardKind::Heading(level),
                title: String::new(),
                body: t.to_string(),
                depth,
            });
            i += 1;
            continue;
        }

        // Thematic break.
        if is_rule(raw) {
            flush_paragraph(&mut para, depth, out);
            out.push(Card {
                kind: CardKind::Rule,
                title: String::new(),
                body: String::new(),
                depth,
            });
            i += 1;
            continue;
        }

        // Table.
        if let Some((table, next)) = parse_table(&lines, i) {
            flush_paragraph(&mut para, depth, out);
            out.push(Card {
                kind: CardKind::Table(table),
                title: String::new(),
                body: String::new(),
                depth,
            });
            i = next;
            continue;
        }

        // Block quote.
        if is_quote_line(raw) {
            flush_paragraph(&mut para, depth, out);
            let mut inner: Vec<String> = Vec::new();
            while i < lines.len() && is_quote_line(lines[i]) {
                inner.push(strip_quote_marker(lines[i]));
                i += 1;
            }
            out.push(Card {
                kind: CardKind::Quote,
                title: String::new(),
                body: inner.join("\n"),
                depth,
            });
            continue;
        }

        if raw.trim().is_empty() {
            flush_paragraph(&mut para, depth, out);
        } else {
            para.push(raw);
        }
        i += 1;
    }
    flush_paragraph(&mut para, depth, out);
}

fn setext_level(line: &str) -> Option<u8> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let t = line.trim();
    if t.len() >= 3 && t.chars().all(|c| c == '=') {
        Some(1)
    } else if t.len() >= 3 && t.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    let Some(first) = t.chars().next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    let mut count = 0usize;
    for c in t.chars() {
        if c == first {
            count += 1;
        } else if c != ' ' && c != '\t' {
            return false;
        }
    }
    count >= 3
}

fn is_quote_line(line: &str) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    indent <= 3 && line.trim_start().starts_with('>')
}

fn strip_quote_marker(line: &str) -> String {
    let t = line.trim_start();
    let rest = t.strip_prefix('>').unwrap_or(t);
    rest.strip_prefix(' ').unwrap_or(rest).to_string()
}

/// Parses a GitHub-style table starting at `lines[start]` (header row, delimiter row, body rows).
/// Returns the table and the index of the first line after it.
fn parse_table(lines: &[&str], start: usize) -> Option<(TableData, usize)> {
    let head = lines.get(start)?;
    let delim = lines.get(start + 1)?;
    if !head.contains('|') || !delim.contains('|') {
        return None;
    }
    let header = split_row(head);
    let align = parse_delimiter(delim)?;
    if header.is_empty() || align.len() != header.len() {
        return None;
    }
    let cols = header.len();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut i = start + 2;
    while i < lines.len() {
        let l = lines[i];
        if l.trim().is_empty() || !l.contains('|') {
            break;
        }
        let mut cells = split_row(l);
        cells.resize(cols, String::new());
        rows.push(cells);
        i += 1;
    }
    Some((TableData { header, align, rows }, i))
}

/// Splits a table row on unescaped `|`, dropping the optional outer pipes.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !t.ends_with('|') {
        cells.push(cur.trim().to_string());
    }
    cells
}

fn parse_delimiter(line: &str) -> Option<Vec<Align>> {
    let cells = split_row(line);
    if cells.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(cells.len());
    for c in &cells {
        let left = c.starts_with(':');
        let right = c.ends_with(':') && c.len() > 1;
        let core = c.trim_start_matches(':').trim_end_matches(':');
        if core.is_empty() || !core.chars().all(|ch| ch == '-') {
            return None;
        }
        out.push(match (left, right) {
            (true, true) => Align::Center,
            (true, false) => Align::Left,
            (false, true) => Align::Right,
            (false, false) => Align::None,
        });
    }
    Some(out)
}

fn flush_paragraph(para: &mut Vec<&str>, depth: usize, out: &mut Vec<Card>) {
    if para.is_empty() {
        return;
    }
    let structured = para.iter().any(|l| is_structured(l));
    let body = if structured {
        para.join("\n")
    } else {
        para.iter().map(|l| l.trim()).collect::<Vec<&str>>().join(" ")
    };
    out.push(Card {
        kind: CardKind::Paragraph,
        title: String::new(),
        body,
        depth,
    });
    para.clear();
}

/// Lines that must keep their own line breaks (lists, quotes, tables).
fn is_structured(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with('>')
        || t.starts_with('|')
        || t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
}

pub(crate) fn heading(line: &str) -> Option<(u8, &str)> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        return None;
    }
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((hashes as u8, text))
}

fn plain(v: &MetaValue) -> String {
    match v {
        MetaValue::Null => String::new(),
        MetaValue::Bool(b) => b.to_string(),
        MetaValue::Int(n) => n.to_string(),
        MetaValue::Str(s) => s.clone(),
        MetaValue::List(items) => items.iter().map(plain).collect::<Vec<String>>().join(", "),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn cards(src: &str) -> Vec<Card> {
        build_cards(&parse(src))
    }

    #[test]
    fn callout_body_is_merged_into_the_card() {
        let c = cards(":::lmf:callout {kind=\"warning\" title=\"Before\"}\nBody text.\n:::\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, CardKind::Callout("warning".to_string()));
        assert_eq!(c[0].title, "Warning \u{2014} Before");
        assert_eq!(c[0].body, "Body text.");
        assert_eq!(c[0].depth, 0);
    }

    #[test]
    fn markdown_blocks() {
        let c = cards("# Title\n\nHello\nworld\n\n- a\n- b\n\n```rust\nfn x() {}\n```\n");
        let kinds: Vec<CardKind> = c.iter().map(|x| x.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                CardKind::Heading(1),
                CardKind::Paragraph,
                CardKind::Paragraph,
                CardKind::Code
            ]
        );
        assert_eq!(c[0].body, "Title");
        assert_eq!(c[1].body, "Hello world");
        assert_eq!(c[2].body, "- a\n- b");
        assert_eq!(c[3].title, "rust");
        assert_eq!(c[3].body, "fn x() {}");
    }

    #[test]
    fn tabs_are_nested_cards() {
        let c = cards(
            ":::lmf:tabs\n:::lmf:tab {title=\"A\"}\na\n:::\n:::lmf:tab {title=\"B\"}\nb\n:::\n:::\n",
        );
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].kind, CardKind::TabGroup);
        assert_eq!((c[1].kind.clone(), c[1].title.as_str(), c[1].depth), (CardKind::Tab, "A", 1));
        assert_eq!(c[2].body, "b");
    }

    #[test]
    fn unknown_extension_shows_source() {
        let c = cards(":::vendor:x\npayload\n:::\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, CardKind::Unknown);
        assert!(c[0].title.contains("vendor:x"));
        assert_eq!(c[0].body, ":::vendor:x\npayload\n:::");
    }

    #[test]
    fn front_matter_becomes_a_meta_card() {
        let c = cards("---\nlmf_version: 1\ntitle: T\ntags: [a, b]\n---\n\nHi\n");
        assert_eq!(c[0].kind, CardKind::Meta);
        assert_eq!(c[0].title, "T");
        assert_eq!(c[0].body, "tags: a, b");
        assert_eq!(c[1].body, "Hi");
    }

    #[test]
    fn unclosed_fence_still_yields_code() {
        let c = cards("```py\nprint(1)\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, CardKind::Code);
        assert_eq!(c[0].body, "print(1)");
    }

    #[test]
    fn math_diagram_and_include_cards() {
        let c = cards(":::lmf:math {format=\"latex\"}\nx^2\n:::\n\n:::lmf:diagram {type=\"mermaid\"}\nA --> B\n:::\n\n:::lmf:include {document=\"Plan\" section=\"Milestones\"}\nSee the plan.\n:::\n");
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].kind, CardKind::Math);
        assert_eq!(c[0].body, "x^2");
        assert_eq!(c[1].kind, CardKind::Diagram);
        assert!(c[1].title.contains("mermaid"));
        assert_eq!(
            c[2].kind,
            CardKind::Include {
                document: "Plan".to_string(),
                section: Some("Milestones".to_string()),
                block: None
            }
        );
        assert_eq!(c[2].body, "See the plan.");
        assert!(c[2].title.contains("Plan"));
    }

    #[test]
    fn cards_from_text_uses_the_given_depth() {
        let c = cards_from_text("# H\n\ntext\n", 2);
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|x| x.depth == 2));
    }

    #[test]
    fn raw_html_is_inert_text() {
        // Policy (ADR-012): HTML is never interpreted. It is shown as the text it is.
        let src = "<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n";
        let c = cards(src);
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|x| x.kind == CardKind::Paragraph));
        assert_eq!(c[0].body, "<script>alert(1)</script>");
        assert_eq!(c[1].body, "<img src=x onerror=alert(1)>");
    }

    #[test]
    fn tables_are_parsed_with_alignment_and_ragged_rows() {
        let c = cards("| Name | Qty | Note |\n|:-----|----:|:----:|\n| a | 1 | x |\n| b | 2 |\n| c \\| d | 3 | y | extra |\n\nAfter\n");
        assert_eq!(c.len(), 2);
        let CardKind::Table(t) = &c[0].kind else {
            panic!("expected table, got {:?}", c[0].kind);
        };
        assert_eq!(t.header, vec!["Name", "Qty", "Note"]);
        assert_eq!(t.align, vec![Align::Left, Align::Right, Align::Center]);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[0], vec!["a", "1", "x"]);
        assert_eq!(t.rows[1], vec!["b", "2", ""]);
        assert_eq!(t.rows[2], vec!["c | d", "3", "y"]);
        assert_eq!(c[1].body, "After");
    }

    #[test]
    fn tables_without_outer_pipes_and_non_tables() {
        let c = cards("a | b\n--|--\n1 | 2\n");
        assert!(matches!(c[0].kind, CardKind::Table(_)));
        // No delimiter row: ordinary text.
        let c = cards("a | b\nc | d\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, CardKind::Paragraph);
        // Column count mismatch between header and delimiter: ordinary text.
        let c = cards("a | b\n--|--|--\n");
        assert_eq!(c[0].kind, CardKind::Paragraph);
    }

    #[test]
    fn quotes_rules_and_setext_headings() {
        let c = cards("> quoted *text*\n> second\n\n---\n\nTitle\n=====\n\nSub\n---\n");
        let kinds: Vec<CardKind> = c.iter().map(|x| x.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                CardKind::Quote,
                CardKind::Rule,
                CardKind::Heading(1),
                CardKind::Heading(2)
            ]
        );
        assert_eq!(c[0].body, "quoted *text*\nsecond");
        assert_eq!(c[2].body, "Title");
        assert_eq!(c[3].body, "Sub");
    }

    #[test]
    fn list_items_and_stars_are_not_rules() {
        let c = cards("- a\n- b\n\n* * *\n\n**bold** text\n");
        let kinds: Vec<CardKind> = c.iter().map(|x| x.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![CardKind::Paragraph, CardKind::Rule, CardKind::Paragraph]
        );
    }

    #[test]
    fn empty_document_has_no_cards() {
        assert!(cards("").is_empty());
    }
}
