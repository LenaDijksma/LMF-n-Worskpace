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
    /// An extension this implementation does not understand; `body` is its source.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub kind: CardKind,
    pub title: String,
    pub body: String,
    /// Nesting depth (0 = top level), for indentation.
    pub depth: usize,
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
    let mut para: Vec<&str> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut code: Vec<&str> = Vec::new();
    let mut code_info = String::new();

    for raw in text.lines() {
        if let Some((ch, len)) = fence {
            if fence_close(raw, ch, len) {
                out.push(Card {
                    kind: CardKind::Code,
                    title: std::mem::take(&mut code_info),
                    body: code.join("\n"),
                    depth,
                });
                code.clear();
                fence = None;
            } else {
                code.push(raw);
            }
            continue;
        }
        if let Some(open) = fence_open(raw) {
            flush_paragraph(&mut para, depth, out);
            fence = Some(open);
            code_info = raw
                .trim()
                .trim_start_matches(|c: char| c == '`' || c == '~')
                .trim()
                .to_string();
            continue;
        }
        if let Some((level, text)) = heading(raw) {
            flush_paragraph(&mut para, depth, out);
            out.push(Card {
                kind: CardKind::Heading(level),
                title: String::new(),
                body: text.to_string(),
                depth,
            });
            continue;
        }
        if raw.trim().is_empty() {
            flush_paragraph(&mut para, depth, out);
        } else {
            para.push(raw);
        }
    }
    if fence.is_some() {
        out.push(Card {
            kind: CardKind::Code,
            title: code_info,
            body: code.join("\n"),
            depth,
        });
    }
    flush_paragraph(&mut para, depth, out);
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

fn heading(line: &str) -> Option<(u8, &str)> {
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
    fn empty_document_has_no_cards() {
        assert!(cards("").is_empty());
    }
}
