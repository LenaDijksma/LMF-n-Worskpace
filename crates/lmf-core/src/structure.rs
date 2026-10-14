//! Structural queries over a document: headings, block IDs and sections.
//!
//! *Block IDs* mark a block so other documents can link to or include it
//! (`[[Plan#block:ms-03]]`, `:::lmf:include {document="Plan" block="ms-03"}`):
//!
//! ```markdown
//! The launch date is fixed. ^launch-date      <- trailing marker: this paragraph
//!
//! | a | b |
//! |---|---|
//! | 1 | 2 |
//! ^table-1                                    <- own line: the block above
//! ```
//!
//! A marker is `^` followed by 2+ characters from `[A-Za-z0-9_-]` (first one alphanumeric),
//! preceded by whitespace or at the start of the line. `x^2` is not a marker. Markers stay in
//! the text, so plain Markdown renderers simply show them.
//!
//! Headings and sections consider *top-level* Markdown only (not text inside extension blocks).

use crate::lines::{fence_close, fence_open, is_blank, split_lines};
use crate::model::{Block, Document};
use crate::preview::heading as atx_heading;
use crate::registry;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    /// Heading text without `#` marks and without a trailing block marker.
    pub text: String,
    /// The heading line.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockId {
    pub id: String,
    /// The `^id` marker itself.
    pub marker: Span,
    /// The block the marker refers to.
    pub block: Span,
}

/// Top-level headings (outside fenced code) in source order.
pub fn headings(doc: &Document) -> Vec<Heading> {
    let mut out = Vec::new();
    for block in &doc.blocks {
        if let Block::Markdown(m) = block {
            scan_headings(doc.text(m.span), m.span.start, &mut out);
        }
    }
    out
}

fn scan_headings(text: &str, base: usize, out: &mut Vec<Heading>) {
    let mut fence: Option<(char, usize)> = None;
    for line in split_lines(text) {
        if let Some((ch, len)) = fence {
            if fence_close(line.text, ch, len) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = fence_open(line.text) {
            fence = Some(open);
            continue;
        }
        if let Some((level, raw)) = atx_heading(line.text) {
            let clean = match trailing_marker(raw) {
                Some((caret, _)) => raw[..caret].trim_end(),
                None => raw,
            };
            out.push(Heading {
                level,
                text: clean.to_string(),
                span: Span::new(base + line.start, base + line.end),
            });
        }
    }
}

/// Index of the first heading whose text equals `name` (case-insensitive, trimmed).
pub fn find_heading(headings: &[Heading], name: &str) -> Option<usize> {
    let wanted = name.trim().to_lowercase();
    headings
        .iter()
        .position(|h| h.text.trim().to_lowercase() == wanted)
}

/// A heading together with everything up to the next heading of the same or a higher level.
pub fn section_span(doc: &Document, headings: &[Heading], index: usize) -> Span {
    let Some(h) = headings.get(index) else {
        return Span::new(0, 0);
    };
    let end_limit = headings[index + 1..]
        .iter()
        .find(|n| n.level <= h.level)
        .map(|n| n.span.start)
        .unwrap_or(doc.source.len());
    let raw = doc.source.get(h.span.start..end_limit).unwrap_or("");
    Span::new(h.span.start, h.span.start + raw.trim_end().len())
}

/// All block IDs (also inside known extension blocks such as callouts), in source order.
pub fn block_ids(doc: &Document) -> Vec<BlockId> {
    let mut out = Vec::new();
    collect_ids(doc, &doc.blocks, &mut out);
    out.sort_by_key(|b| b.marker.start);
    out
}

fn collect_ids(doc: &Document, blocks: &[Block], out: &mut Vec<BlockId>) {
    for block in blocks {
        match block {
            Block::Markdown(m) => scan_ids(doc.text(m.span), m.span.start, out),
            Block::Directive(d) => {
                if registry::lookup(&d.namespace, &d.name).is_some() {
                    collect_ids(doc, &d.body, out);
                }
            }
        }
    }
}

fn extend(para: &mut Option<(usize, usize)>, start: usize, end: usize) {
    *para = Some(match *para {
        Some((s, _)) => (s, end),
        None => (start, end),
    });
}

fn scan_ids(text: &str, base: usize, out: &mut Vec<BlockId>) {
    let mut fence: Option<(char, usize)> = None;
    let mut para: Option<(usize, usize)> = None;
    let mut last_para: Option<(usize, usize)> = None;

    for line in split_lines(text) {
        let ls = base + line.start;
        let le = base + line.end;
        if let Some((ch, len)) = fence {
            if fence_close(line.text, ch, len) {
                fence = None;
            }
            extend(&mut para, ls, le);
            continue;
        }
        if let Some(open) = fence_open(line.text) {
            fence = Some(open);
            extend(&mut para, ls, le);
            continue;
        }
        if is_blank(line.text) {
            if para.is_some() {
                last_para = para.take();
            }
            continue;
        }
        match trailing_marker(line.text) {
            Some((0, id)) => {
                // A marker on its own line refers to the paragraph above it.
                let target = para.take().or(last_para);
                if let Some((s, e)) = target {
                    let marker_end = ls + line.text.trim_end().len();
                    out.push(BlockId {
                        id: id.to_string(),
                        marker: Span::new(ls, marker_end),
                        block: Span::new(s, e),
                    });
                }
                last_para = target;
            }
            Some((caret, id)) => {
                extend(&mut para, ls, le);
                let start = para.map(|p| p.0).unwrap_or(ls);
                let marker_end = ls + line.text.trim_end().len();
                out.push(BlockId {
                    id: id.to_string(),
                    marker: Span::new(ls + caret, marker_end),
                    block: Span::new(start, le),
                });
            }
            None => extend(&mut para, ls, le),
        }
    }
}

/// The text of a block with its `^id` marker removed.
pub fn block_text(doc: &Document, b: &BlockId) -> String {
    let mut text = doc.text(b.block).replace("\r\n", "\n");
    if b.marker.start >= b.block.start && b.marker.end <= b.block.end {
        // Marker sits inside the block (trailing marker): cut it out.
        let from = b.marker.start - b.block.start;
        let to = b.marker.end - b.block.start;
        let raw = doc.text(b.block);
        if let (Some(head), Some(tail)) = (raw.get(..from), raw.get(to..)) {
            text = format!("{}{}", head.trim_end_matches([' ', '\t']), tail).replace("\r\n", "\n");
        }
    }
    text.trim().to_string()
}

/// Finds a trailing `^id` marker in a line. Returns the byte offset of the `^` and the id.
pub(crate) fn trailing_marker(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_end();
    let caret = t.rfind('^')?;
    let id = &t[caret + 1..];
    if !is_block_id(id) {
        return None;
    }
    if caret > 0 {
        let prev = t[..caret].chars().next_back()?;
        if !prev.is_whitespace() {
            return None;
        }
    }
    Some((caret, id))
}

/// Whether `s` is a syntactically valid block id.
pub fn is_block_id(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    s.len() >= 2 && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn headings_ignore_code_and_strip_markers() {
        let doc = parse("# One\n\n```\n# not a heading\n```\n\n## Two ^two-id\n\ntext\n");
        let h = headings(&doc);
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].level, h[0].text.as_str()), (1, "One"));
        assert_eq!((h[1].level, h[1].text.as_str()), (2, "Two"));
        assert_eq!(doc.text(h[0].span), "# One");
    }

    #[test]
    fn trailing_and_standalone_markers() {
        let src = "First paragraph. ^first\n\nSecond line one\nsecond line two ^second\n\n| a |\n|---|\n| 1 |\n^table-1\n\nIgnored x^2 and ^ no\n";
        let doc = parse(src);
        let ids = block_ids(&doc);
        let names: Vec<&str> = ids.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(names, vec!["first", "second", "table-1"]);
        assert_eq!(block_text(&doc, &ids[0]), "First paragraph.");
        assert_eq!(block_text(&doc, &ids[1]), "Second line one\nsecond line two");
        assert_eq!(block_text(&doc, &ids[2]), "| a |\n|---|\n| 1 |");
    }

    #[test]
    fn standalone_marker_after_blank_line_refers_to_previous_paragraph() {
        let doc = parse("Alpha beta\n\n^alpha-id\n\nNext\n");
        let ids = block_ids(&doc);
        assert_eq!(ids.len(), 1);
        assert_eq!(block_text(&doc, &ids[0]), "Alpha beta");
    }

    #[test]
    fn markers_inside_code_fences_are_ignored() {
        let doc = parse("```\ncode ^not-an-id\n```\n");
        assert!(block_ids(&doc).is_empty());
    }

    #[test]
    fn ids_inside_known_extensions_are_found() {
        let doc = parse(":::lmf:callout\nImportant ^imp\n:::\n\n:::vendor:x\nhidden ^hid\n:::\n");
        let ids = block_ids(&doc);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].id, "imp");
    }

    #[test]
    fn sections_run_to_the_next_heading_of_same_or_higher_level() {
        let src = "# Top\n\nintro\n\n## A\n\na text\n\n### A1\n\nsub\n\n## B\n\nb text\n";
        let doc = parse(src);
        let h = headings(&doc);
        let a = find_heading(&h, "a").unwrap();
        let text = doc.text(section_span(&doc, &h, a));
        assert!(text.starts_with("## A"));
        assert!(text.contains("### A1"));
        assert!(text.contains("sub"));
        assert!(!text.contains("b text"));
        let top = find_heading(&h, "Top").unwrap();
        assert!(doc.text(section_span(&doc, &h, top)).contains("b text"));
        assert_eq!(find_heading(&h, "missing"), None);
    }

    #[test]
    fn id_syntax() {
        assert!(is_block_id("ms-03"));
        assert!(is_block_id("a1"));
        assert!(!is_block_id("a"));
        assert!(!is_block_id("-ab"));
        assert!(!is_block_id("a b"));
    }
}
