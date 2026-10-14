//! Internal links: wikilinks (`[[Target]]`) and relative Markdown links to other notes.
//!
//! Extraction is pure and syntactic. *Resolution* (which document a link points at) needs the
//! set of documents in a workspace and lives in the `lmf-workspace` crate.
//!
//! Recognized forms:
//!
//! ```text
//! [[Project Plan]]                 target
//! [[Project Plan|read the plan]]   target + display text
//! [[Project Plan#Milestones]]      target + heading fragment
//! [[Project Plan#block:ms-03]]     target + block-id fragment
//! [text](notes/plan.md#Intro)      relative Markdown link to .md / .lmf / .markdown
//! ```
//!
//! Links inside fenced code, inline code and unknown extension blocks are ignored.

use crate::lines::{fence_close, fence_open};
use crate::model::{Block, Document};
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    Wiki,
    Markdown,
    /// An `http(s)` link. Only produced by [`extract_all_links`]; never fetched or opened by
    /// this crate.
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment {
    None,
    Heading(String),
    Block(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// Document name or relative path, without fragment.
    pub target: String,
    pub fragment: Fragment,
    /// Display text, if given.
    pub label: Option<String>,
    /// Location of the whole link in the document source.
    pub span: Span,
}

/// All internal links in a document, in source order.
pub fn extract_links(doc: &Document) -> Vec<Link> {
    extract(doc, false)
}

/// Internal links plus external `http(s)` links (`[text](https://…)` and `<https://…>`).
pub fn extract_all_links(doc: &Document) -> Vec<Link> {
    extract(doc, true)
}

fn extract(doc: &Document, include_external: bool) -> Vec<Link> {
    let mut out = Vec::new();
    collect(doc, &doc.blocks, &mut out, include_external);
    out.sort_by_key(|l| l.span.start);
    out
}

fn collect(doc: &Document, blocks: &[Block], out: &mut Vec<Link>, ext: bool) {
    for block in blocks {
        match block {
            Block::Markdown(m) => scan_markdown(doc.text(m.span), m.span.start, out, ext),
            Block::Directive(d) => {
                // Only known extensions have a body we understand; unknown blocks are opaque.
                if crate::registry::lookup(&d.namespace, &d.name).is_some() {
                    collect(doc, &d.body, out, ext);
                }
            }
        }
    }
}

fn scan_markdown(text: &str, base: usize, out: &mut Vec<Link>, ext: bool) {
    let mut fence: Option<(char, usize)> = None;
    let mut offset = 0usize;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches(|c| c == '\n' || c == '\r');
        let line_start = base + offset;
        offset += raw.len();

        if let Some((ch, len)) = fence {
            if fence_close(line, ch, len) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = fence_open(line) {
            fence = Some(open);
            continue;
        }
        scan_line(line, line_start, out, ext);
    }
}

fn scan_line(line: &str, line_start: usize, out: &mut Vec<Link>, ext: bool) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                // Skip an inline code span: matching run of backticks of the same length.
                let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                let after = i + run;
                match find_run(bytes, after, run) {
                    Some(end) => i = end + run,
                    None => i = after,
                }
            }
            b'\\' => i += 2,
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                if let Some(close) = line[i + 2..].find("]]") {
                    let inner = &line[i + 2..i + 2 + close];
                    let end = i + 2 + close + 2;
                    if let Some(link) = parse_wiki(inner, line_start + i, line_start + end) {
                        out.push(link);
                    }
                    i = end;
                } else {
                    i += 2;
                }
            }
            b'<' => {
                let mut advanced = false;
                if ext {
                    if let Some(close) = line[i + 1..].find('>') {
                        let url = &line[i + 1..i + 1 + close];
                        if is_http_url(url) {
                            out.push(Link {
                                kind: LinkKind::External,
                                target: url.to_string(),
                                fragment: Fragment::None,
                                label: None,
                                span: Span::new(line_start + i, line_start + i + close + 2),
                            });
                            i += close + 2;
                            advanced = true;
                        }
                    }
                }
                if !advanced {
                    i += 1;
                }
            }
            b'[' => match parse_md_link(line, i, line_start, ext) {
                Some((link, end)) => {
                    out.push(link);
                    i = end;
                }
                None => i += 1,
            },
            _ => i += 1,
        }
    }
}

fn find_run(bytes: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let n = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if n == run {
                return Some(i);
            }
            i += n;
        } else {
            i += 1;
        }
    }
    None
}

pub(crate) fn parse_wiki(inner: &str, start: usize, end: usize) -> Option<Link> {
    if inner.trim().is_empty() || inner.contains('[') {
        return None;
    }
    let (target_part, label) = match inner.split_once('|') {
        Some((t, l)) => (t, Some(l.trim().to_string()).filter(|l| !l.is_empty())),
        None => (inner, None),
    };
    let (target, fragment) = split_fragment(target_part.trim());
    // `[[#Heading]]` (empty target) links within the current document.
    if target.is_empty() && matches!(fragment, Fragment::None) {
        return None;
    }
    Some(Link {
        kind: LinkKind::Wiki,
        target: target.to_string(),
        fragment,
        label,
        span: Span::new(start, end),
    })
}

pub(crate) fn split_fragment(s: &str) -> (&str, Fragment) {
    match s.split_once('#') {
        None => (s.trim(), Fragment::None),
        Some((t, f)) => {
            let f = f.trim();
            let fragment = if f.is_empty() {
                Fragment::None
            } else if let Some(id) = f.strip_prefix("block:") {
                Fragment::Block(id.trim().to_string())
            } else {
                Fragment::Heading(f.to_string())
            };
            (t.trim(), fragment)
        }
    }
}

/// Parses `[label](dest)` starting at `line[start] == '['`. Returns the link and the index just
/// past it. Only relative destinations to note files are reported.
fn parse_md_link(line: &str, start: usize, line_start: usize, ext: bool) -> Option<(Link, usize)> {
    let rest = &line[start + 1..];
    let label_end = rest.find("](")?;
    let label = &rest[..label_end];
    if label.contains('[') {
        return None;
    }
    let dest_start = start + 1 + label_end + 2;
    let dest_len = line[dest_start..].find(')')?;
    let dest = line[dest_start..dest_start + dest_len].trim();
    let end = dest_start + dest_len + 1;

    let dest = dest.split_whitespace().next().unwrap_or("");
    if dest.is_empty() {
        return None;
    }
    if is_external(dest) {
        if ext && is_http_url(dest) {
            return Some((
                Link {
                    kind: LinkKind::External,
                    target: dest.to_string(),
                    fragment: Fragment::None,
                    label: Some(label.trim().to_string()).filter(|l| !l.is_empty()),
                    span: Span::new(line_start + start, line_start + end),
                },
                end,
            ));
        }
        return None;
    }
    let (path, fragment) = split_fragment(dest);
    if !is_note_path(path) {
        return None;
    }
    Some((
        Link {
            kind: LinkKind::Markdown,
            target: path.to_string(),
            fragment,
            label: Some(label.trim().to_string()).filter(|l| !l.is_empty()),
            span: Span::new(line_start + start, line_start + end),
        },
        end,
    ))
}

pub(crate) fn is_http_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && lower.len() > 8
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub(crate) fn is_external(dest: &str) -> bool {
    let lower = dest.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("//")
        || lower.contains("://")
}

pub(crate) fn is_note_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".lmf") || lower.ends_with(".markdown")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn links(src: &str) -> Vec<Link> {
        extract_links(&parse(src))
    }

    #[test]
    fn wikilink_forms() {
        let l = links("See [[Project Plan]] and [[Project Plan|the plan]] and [[Project Plan#Milestones]] and [[Project Plan#block:ms-03]].");
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].target, "Project Plan");
        assert_eq!(l[0].fragment, Fragment::None);
        assert_eq!(l[1].label.as_deref(), Some("the plan"));
        assert_eq!(l[2].fragment, Fragment::Heading("Milestones".into()));
        assert_eq!(l[3].fragment, Fragment::Block("ms-03".into()));
        assert!(l.iter().all(|x| x.kind == LinkKind::Wiki));
    }

    #[test]
    fn spans_point_at_the_link_text() {
        let src = "x [[A|b]] y";
        let doc = parse(src);
        let l = extract_links(&doc);
        assert_eq!(doc.text(l[0].span), "[[A|b]]");
    }

    #[test]
    fn markdown_links_to_notes_only() {
        let l = links("[a](notes/plan.md#Intro) [b](https://x.org/a.md) [c](image.png) [d](other.lmf)");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].target, "notes/plan.md");
        assert_eq!(l[0].fragment, Fragment::Heading("Intro".into()));
        assert_eq!(l[1].target, "other.lmf");
        assert!(l.iter().all(|x| x.kind == LinkKind::Markdown));
    }

    #[test]
    fn code_is_ignored() {
        let l = links("`[[inline]]` and\n\n```\n[[fenced]]\n```\n\n[[real]]\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "real");
    }

    #[test]
    fn escaped_and_malformed_are_ignored() {
        assert!(links("\\[[escaped]] [[]] [[ ]] [[unclosed").is_empty());
    }

    #[test]
    fn links_inside_known_extensions_count_but_not_unknown() {
        let l = links(":::lmf:callout\nsee [[A]]\n:::\n\n:::vendor:x\n[[B]]\n:::\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "A");
    }

    #[test]
    fn external_links_only_with_extract_all() {
        let doc = parse("[site](https://example.org/a?b=1) <https://x.org> [mail](mailto:a@b.c) [n](note.md)");
        let internal = extract_links(&doc);
        assert_eq!(internal.len(), 1);
        assert_eq!(internal[0].target, "note.md");

        let all = extract_all_links(&doc);
        let kinds: Vec<LinkKind> = all.iter().map(|l| l.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![LinkKind::External, LinkKind::External, LinkKind::Markdown]
        );
        assert_eq!(all[0].target, "https://example.org/a?b=1");
        assert_eq!(all[0].label.as_deref(), Some("site"));
        assert_eq!(all[1].target, "https://x.org");
        assert_eq!(parse("<https://x.org>").source.len(), 15);
    }

    #[test]
    fn dangerous_schemes_are_never_external_links() {
        let doc = parse("[a](javascript:alert(1)) [b](file:///etc/passwd) <javascript:x>");
        assert!(extract_all_links(&doc).is_empty());
    }

    #[test]
    fn unicode_does_not_panic() {
        let l = links("é [[日本語|ラベル]] 😀 [x](ü.md)");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].target, "日本語");
    }
}
