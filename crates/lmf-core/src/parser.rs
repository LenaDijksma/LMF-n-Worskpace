//! The LMF parser.
//!
//! Single pass over the lines of the input with an explicit stack of open directives, so
//! nesting is handled by construction rather than by regular expressions. The parser never
//! evaluates, fetches or executes anything.
//!
//! ## Directive grammar (LMF 0.1 draft)
//!
//! ```text
//! opener   = 0*3SP 3*COLON name [ 1*WS attr-block ] *WS EOL
//! closer   = 0*3SP 3*COLON *WS EOL                      ; bare: colons only
//! name     = namespace ":" ident                        ; lowercase, e.g. lmf:callout
//! ident    = LOWER *( LOWER / DIGIT / "_" / "-" )
//! ```
//!
//! * A closer closes the innermost open directive if it has **at least as many colons** as
//!   that directive's opener. A shorter colon-only line is plain content, so an author can
//!   wrap literal `:::` lines by using a longer outer fence.
//! * Lines inside fenced code blocks (``` or ~~~) are never directive delimiters.
//! * Delimiters indented 4+ spaces are indented code, not delimiters.
//! * A `:::word` line whose name is not a valid `namespace:name` is plain Markdown (and is
//!   reported with a warning, so e.g. Docusaurus-style `:::note` is not silently misread).
//! * Unclosed directives are closed at the end of input and reported; their content is kept.

use crate::attrs::parse_attr_block;
use crate::diagnostics::{codes, sort_diagnostics, Diagnostic};
use crate::frontmatter::detect_front_matter;
use crate::lines::{fence_close, fence_open, indent_of, is_blank, split_lines, Line};
use crate::model::{Block, Directive, Document, MarkdownBlock};
use crate::span::Span;

/// Parses LMF source text. Never fails: any text is a valid document; problems are reported
/// in [`Document::diagnostics`].
pub fn parse(src: &str) -> Document {
    let lines = split_lines(src);
    let mut diags: Vec<Diagnostic> = Vec::new();

    let (front_matter, first_line) = match detect_front_matter(&lines, &mut diags) {
        Some((fm, consumed)) => (Some(fm), consumed),
        None => (None, 0),
    };

    let mut stack: Vec<Frame> = vec![Frame::new(None)];
    let mut fence: Option<(char, usize, Span)> = None;

    for idx in first_line..lines.len() {
        let line = lines[idx];

        if let Some((ch, len, _)) = fence {
            if fence_close(line.text, ch, len) {
                fence = None;
            }
            push_md(&mut stack, idx);
            continue;
        }
        if let Some((ch, len)) = fence_open(line.text) {
            fence = Some((ch, len, Span::new(line.start, line.end)));
            push_md(&mut stack, idx);
            continue;
        }

        match classify(line.text) {
            LineKind::Text => push_md(&mut stack, idx),

            LineKind::BadOpener { token } => {
                diags.push(Diagnostic::warning(
                    codes::MALFORMED_DIRECTIVE,
                    format!(
                        "`{}` is not a valid directive name (expected lowercase `namespace:name`); treated as Markdown text",
                        token
                    ),
                    Span::new(line.start, line.end),
                ));
                push_md(&mut stack, idx);
            }

            LineKind::Closer { colons } => {
                let can_close = stack.len() > 1
                    && stack
                        .last()
                        .and_then(|f| f.directive.as_ref())
                        .map(|d| colons >= d.colons)
                        .unwrap_or(false);
                if can_close {
                    close_top(&mut stack, &lines, Some(Span::new(line.start, line.end)), 0);
                } else {
                    if stack.len() == 1 {
                        diags.push(Diagnostic::warning(
                            codes::MALFORMED_DIRECTIVE,
                            "closing `:::` without a matching open directive; treated as Markdown text",
                            Span::new(line.start, line.end),
                        ));
                    }
                    push_md(&mut stack, idx);
                }
            }

            LineKind::Opener {
                colons,
                namespace,
                name,
                rest,
                rest_offset,
            } => {
                if let Some(top) = stack.last_mut() {
                    flush_md(top, &lines);
                }
                let open_span = Span::new(line.start, line.end);
                let mut dir = Directive {
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                    colons,
                    attrs: Vec::new(),
                    attrs_valid: true,
                    body: Vec::new(),
                    closed: false,
                    span: open_span,
                    open_span,
                    close_span: None,
                };

                let rest_trimmed = rest.trim_end();
                let attr_text = rest_trimmed.trim_start();
                let lead_ws = rest_trimmed.len() - attr_text.len();
                let attr_base = line.start + rest_offset + lead_ws;
                if !attr_text.is_empty() {
                    if attr_text.starts_with('{') {
                        let parsed = parse_attr_block(attr_text, attr_base);
                        dir.attrs = parsed.attrs;
                        for (key, span) in parsed.duplicates {
                            diags.push(Diagnostic::error(
                                codes::DUPLICATE_ATTRIBUTE,
                                format!("attribute `{}` is given more than once; the first value is used", key),
                                span,
                            ));
                        }
                        if let Some(e) = parsed.error {
                            dir.attrs_valid = false;
                            let end = (e.offset + 1).min(line.end).max(e.offset);
                            diags.push(Diagnostic::error(
                                codes::INVALID_ATTRIBUTE_SYNTAX,
                                e.message,
                                Span::new(e.offset, end),
                            ));
                        }
                    } else {
                        dir.attrs_valid = false;
                        diags.push(Diagnostic::error(
                            codes::INVALID_ATTRIBUTE_SYNTAX,
                            "unexpected text after the directive name; attributes must be written as `{key=\"value\"}`",
                            Span::new(attr_base, line.end),
                        ));
                    }
                }
                stack.push(Frame::new(Some(dir)));
            }
        }
    }

    if let Some((_, _, span)) = fence {
        diags.push(Diagnostic::warning(
            codes::UNCLOSED_FENCE,
            "code fence is never closed; the rest of the document is treated as code",
            span,
        ));
    }

    let doc_end = lines.last().map(|l| l.end).unwrap_or(0);
    while stack.len() > 1 {
        if let Some(top) = stack.last().and_then(|f| f.directive.as_ref()) {
            diags.push(Diagnostic::error(
                codes::MALFORMED_DIRECTIVE,
                format!(
                    "directive `{}` opened here is never closed; expected a closing `:::`",
                    top.qualified_name()
                ),
                top.open_span,
            ));
        }
        close_top(&mut stack, &lines, None, doc_end);
    }
    if let Some(root) = stack.first_mut() {
        flush_md(root, &lines);
    }
    let blocks = stack.pop().map(|f| f.children).unwrap_or_default();

    sort_diagnostics(&mut diags);
    Document {
        source: src.to_string(),
        front_matter,
        blocks,
        diagnostics: diags,
    }
}

/// Parses raw bytes. Invalid UTF-8 is reported as `LMF001`; for analysis the invalid bytes
/// are replaced with U+FFFD, so callers must not write such a document back.
pub fn parse_bytes(bytes: &[u8]) -> Document {
    match std::str::from_utf8(bytes) {
        Ok(s) => parse(s),
        Err(e) => {
            let lossy = String::from_utf8_lossy(bytes).into_owned();
            let mut doc = parse(&lossy);
            let at = e.valid_up_to();
            let end = (at + 1).min(doc.source.len()).max(at);
            doc.diagnostics.push(Diagnostic::error(
                codes::INVALID_UTF8,
                "invalid UTF-8 byte sequence; invalid bytes were replaced with U+FFFD for analysis",
                Span::new(at, end),
            ));
            sort_diagnostics(&mut doc.diagnostics);
            doc
        }
    }
}

// ---------------------------------------------------------------------------------------
// internals

/// A region being built: the document root (`directive == None`) or an open directive.
struct Frame {
    directive: Option<Directive>,
    children: Vec<Block>,
    /// Inclusive range of line indices of pending, not yet flushed Markdown.
    md: Option<(usize, usize)>,
}

impl Frame {
    fn new(directive: Option<Directive>) -> Self {
        Frame {
            directive,
            children: Vec::new(),
            md: None,
        }
    }
}

fn push_md(stack: &mut [Frame], idx: usize) {
    if let Some(top) = stack.last_mut() {
        top.md = Some(match top.md {
            Some((first, _)) => (first, idx),
            None => (idx, idx),
        });
    }
}

fn flush_md(frame: &mut Frame, lines: &[Line<'_>]) {
    if let Some((first, last)) = frame.md.take() {
        let mut a = first;
        while a <= last && is_blank(lines[a].text) {
            a += 1;
        }
        if a > last {
            return;
        }
        let mut b = last;
        while b > a && is_blank(lines[b].text) {
            b -= 1;
        }
        frame.children.push(Block::Markdown(MarkdownBlock {
            span: Span::new(lines[a].start, lines[b].end),
        }));
    }
}

/// Pops the innermost open directive, finishes it and appends it to its parent.
fn close_top(stack: &mut Vec<Frame>, lines: &[Line<'_>], close: Option<Span>, doc_end: usize) {
    if stack.len() < 2 {
        return;
    }
    if let Some(top) = stack.last_mut() {
        flush_md(top, lines);
    }
    let Some(frame) = stack.pop() else {
        return;
    };
    let Some(mut dir) = frame.directive else {
        return;
    };
    dir.body = frame.children;
    match close {
        Some(span) => {
            dir.closed = true;
            dir.close_span = Some(span);
            dir.span = Span::new(dir.span.start, span.end);
        }
        None => {
            dir.closed = false;
            dir.span = Span::new(dir.span.start, doc_end.max(dir.span.end));
        }
    }
    if let Some(parent) = stack.last_mut() {
        parent.children.push(Block::Directive(dir));
    }
}

enum LineKind<'a> {
    Text,
    Closer {
        colons: usize,
    },
    Opener {
        colons: usize,
        namespace: &'a str,
        name: &'a str,
        rest: &'a str,
        /// Offset of `rest` within the line.
        rest_offset: usize,
    },
    BadOpener {
        token: &'a str,
    },
}

fn classify(text: &str) -> LineKind<'_> {
    let indent = indent_of(text);
    if indent > 3 {
        return LineKind::Text;
    }
    let body = &text[indent..];
    let colons = body.bytes().take_while(|&b| b == b':').count();
    if colons < 3 {
        return LineKind::Text;
    }
    let after = &body[colons..];
    if after.trim().is_empty() {
        return LineKind::Closer { colons };
    }
    let first = match after.chars().next() {
        Some(c) => c,
        None => return LineKind::Text,
    };
    if first.is_whitespace() || first == '{' {
        return LineKind::Text;
    }
    let token_len = after
        .find(|c: char| c.is_whitespace() || c == '{')
        .unwrap_or(after.len());
    let token = &after[..token_len];
    let rest = &after[token_len..];
    match split_qualified(token) {
        Some((namespace, name)) => LineKind::Opener {
            colons,
            namespace,
            name,
            rest,
            rest_offset: indent + colons + token_len,
        },
        None => LineKind::BadOpener { token },
    }
}

fn split_qualified(token: &str) -> Option<(&str, &str)> {
    let (namespace, name) = token.split_once(':')?;
    if is_ident(namespace) && is_ident(name) {
        Some((namespace, name))
    } else {
        None
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_directive(doc: &Document) -> &Directive {
        match &doc.blocks[0] {
            Block::Directive(d) => d,
            other => panic!("expected directive, got {:?}", other),
        }
    }

    #[test]
    fn plain_markdown_is_one_block() {
        let src = "# Title\n\nSome *text*.\n\n- a\n- b\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty());
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.text(doc.blocks[0].span()), "# Title\n\nSome *text*.\n\n- a\n- b");
        assert_eq!(doc.to_source(), src);
    }

    #[test]
    fn simple_callout() {
        let src = ":::lmf:callout {kind=\"warning\" title=\"Read this\"}\nBody **text**.\n:::\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        let d = only_directive(&doc);
        assert_eq!(d.qualified_name(), "lmf:callout");
        assert!(d.closed);
        assert_eq!(d.attr_str("kind"), Some("warning"));
        assert_eq!(d.attr_str("title"), Some("Read this"));
        assert_eq!(d.body.len(), 1);
        assert_eq!(doc.text(d.body[0].span()), "Body **text**.");
        assert_eq!(d.span.start, 0);
        assert_eq!(d.span.end, src.len() - 1);
    }

    #[test]
    fn nested_same_length_directives() {
        let src = ":::lmf:tabs\n:::lmf:tab {title=\"A\"}\na\n:::\n:::lmf:tab {title=\"B\"}\nb\n:::\n:::\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        let tabs = only_directive(&doc);
        assert_eq!(tabs.body.len(), 2);
        assert_eq!(doc.directives().len(), 3);
    }

    #[test]
    fn longer_outer_fence_escapes_inner_colons() {
        let src = "::::lmf:callout\n:::\nliteral\n::::\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        let d = only_directive(&doc);
        assert!(d.closed);
        assert_eq!(doc.text(d.body[0].span()), ":::\nliteral");
    }

    #[test]
    fn delimiters_inside_code_fences_are_text() {
        let src = "```md\n:::lmf:callout\nx\n:::\n```\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty());
        assert_eq!(doc.blocks.len(), 1);
        assert!(matches!(doc.blocks[0], Block::Markdown(_)));
    }

    #[test]
    fn unclosed_directive_keeps_content() {
        let src = "before\n\n:::lmf:callout\nstill here\n";
        let doc = parse(src);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(doc.diagnostics[0].code, "LMF007");
        assert_eq!(doc.blocks.len(), 2);
        match &doc.blocks[1] {
            Block::Directive(d) => {
                assert!(!d.closed);
                assert_eq!(doc.text(d.body[0].span()), "still here");
            }
            _ => panic!("expected directive"),
        }
    }

    #[test]
    fn unknown_extension_is_preserved_as_directive() {
        let src = ":::vendor:interactive-map {version=\"3\"}\npayload { not: parsed }\n:::\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty());
        let d = only_directive(&doc);
        assert_eq!(d.qualified_name(), "vendor:interactive-map");
        assert_eq!(doc.text(d.span), src.trim_end());
    }

    #[test]
    fn bad_attribute_syntax_keeps_the_block() {
        let doc = parse(":::lmf:callout {kind=warning}\nx\n:::\n");
        let codes: Vec<&str> = doc.diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, vec!["LMF008"]);
        let d = only_directive(&doc);
        assert!(!d.attrs_valid);
        assert!(d.closed);
    }

    #[test]
    fn docusaurus_style_is_markdown_with_warnings() {
        let doc = parse(":::note\nhello\n:::\n");
        let codes: Vec<&str> = doc.diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, vec!["LMF007", "LMF007"]);
        assert_eq!(doc.blocks.len(), 1);
        assert!(matches!(doc.blocks[0], Block::Markdown(_)));
    }

    #[test]
    fn indented_delimiters_are_code() {
        let doc = parse("    :::lmf:callout\n    x\n");
        assert!(doc.diagnostics.is_empty());
        assert!(matches!(doc.blocks[0], Block::Markdown(_)));
    }

    #[test]
    fn crlf_input_is_handled() {
        let src = ":::lmf:callout\r\nbody\r\n:::\r\n";
        let doc = parse(src);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert!(only_directive(&doc).closed);
        assert_eq!(doc.to_source(), src);
    }

    #[test]
    fn invalid_utf8_is_reported() {
        let doc = parse_bytes(b"ok\n\xff\xfe\n");
        assert!(doc.diagnostics.iter().any(|d| d.code == "LMF001"));
    }

    #[test]
    fn empty_input() {
        let doc = parse("");
        assert!(doc.blocks.is_empty());
        assert!(doc.diagnostics.is_empty());
    }
}
