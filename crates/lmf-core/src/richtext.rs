//! Preparing Markdown text for a rich-text widget that understands only part of Markdown.
//!
//! A front end such as Slint's `StyledText` renders bold, italic, strikethrough, inline code,
//! links and lists, but not wikilinks, task lists, images or raw HTML. [`display_markdown`]
//! rewrites a piece of Markdown so that such a widget shows it sensibly and *safely*:
//!
//! * `[[wikilinks]]` and relative links to notes become ordinary links whose destination the
//!   front end chooses (so clicks can be routed internally), or struck-through text if the target
//!   does not exist;
//! * `http(s)` links are kept; every other scheme (`javascript:`, `file:`, `mailto:` …) becomes
//!   plain text, never a link;
//! * raw HTML is neutralised: every `<` is escaped, so tags are shown as text, never interpreted;
//! * images become an italic `(image: alt)` placeholder (real images are a separate feature);
//! * task list items get ☐ / ☑ markers.
//!
//! Code spans and backslash escapes are left untouched.

use crate::links::{is_external, is_http_url, is_note_path, parse_wiki, split_fragment};
use crate::links::{Fragment, Link, LinkKind};
use crate::span::Span;

/// How the front end wants an internal link to be displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Use this string as the link destination (e.g. `lmf:7`).
    Href(String),
    /// The target does not exist (or cannot be resolved): show the label struck through.
    Unresolved,
}

/// Rewrites `text` for display. `resolver` is called once per internal link, in source order.
pub fn display_markdown(
    text: &str,
    resolver: &mut dyn FnMut(&Link) -> LinkTarget,
) -> String {
    let text = task_markers(text);
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 16);
    let mut i = 0usize;

    while i < b.len() {
        match b[i] {
            b'\\' => {
                out.push('\\');
                i += 1;
                if let Some(ch) = text[i..].chars().next() {
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
            b'`' => {
                let run = b[i..].iter().take_while(|&&x| x == b'`').count();
                match find_closing_run(b, i + run, run) {
                    Some(end) => {
                        out.push_str(&text[i..end + run]);
                        i = end + run;
                    }
                    None => {
                        out.push_str(&text[i..i + run]);
                        i += run;
                    }
                }
            }
            b'!' if b.get(i + 1) == Some(&b'[') => match bracket_link(&text, i + 1) {
                Some(bl) => {
                    let alt = if bl.label.trim().is_empty() {
                        bl.dest
                    } else {
                        bl.label.trim()
                    };
                    out.push_str(&format!("*(image: {})*", escape(alt)));
                    i = bl.end;
                }
                None => {
                    out.push('!');
                    i += 1;
                }
            },
            b'[' if b.get(i + 1) == Some(&b'[') => match wiki_at(&text, i) {
                Some((link, end)) => {
                    let label = wiki_label(&link);
                    push_internal(&mut out, &link, &escape(&label), resolver);
                    i = end;
                }
                None => {
                    out.push_str("\\[");
                    i += 1;
                }
            },
            b'[' => match bracket_link(&text, i) {
                Some(bl) => {
                    push_md_link(&mut out, &bl, resolver);
                    i = bl.end;
                }
                None => {
                    out.push_str("\\[");
                    i += 1;
                }
            },
            b'<' => {
                let rest = &text[i + 1..];
                let close = rest.find(|c: char| c == '>' || c == '\n' || c == ' ');
                match close {
                    Some(k) if rest.as_bytes()[k] == b'>' && is_http_url(&rest[..k]) => {
                        let url = &rest[..k];
                        out.push_str(&format!("[{}]({})", escape(url), url));
                        i += k + 2;
                    }
                    _ => {
                        out.push_str("\\<");
                        i += 1;
                    }
                }
            }
            _ => {
                if let Some(ch) = text[i..].chars().next() {
                    out.push(ch);
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
        }
    }
    out
}

/// Destination used for internal links in rich text: `lmf:` followed by the percent-encoded
/// workspace path of the note. Self-describing, so rendered text never goes stale.
pub fn encode_href(path: &str) -> String {
    let mut out = String::from("lmf:");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Inverse of [`encode_href`]. Returns `None` for anything that is not a well-formed `lmf:` href.
pub fn decode_href(url: &str) -> Option<String> {
    let rest = url.strip_prefix("lmf:")?;
    let b = rest.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = hex_value(*b.get(i + 1)?)?;
            let lo = hex_value(*b.get(i + 2)?)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

struct BracketLink<'a> {
    label: &'a str,
    dest: &'a str,
    /// Offset just past the closing `)`.
    end: usize,
}

/// Parses `[label](dest)` at `start` (where `text[start] == '['`), on a single line.
fn bracket_link(text: &str, start: usize) -> Option<BracketLink<'_>> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut i = start;
    let mut label_end = None;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'\n' => return None,
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    label_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let label_end = label_end?;
    if b.get(label_end + 1) != Some(&b'(') {
        return None;
    }
    let dest_start = label_end + 2;
    let mut d = 1usize;
    let mut j = dest_start;
    while j < b.len() {
        match b[j] {
            b'\n' => return None,
            b'(' => d += 1,
            b')' => {
                d -= 1;
                if d == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= b.len() {
        return None;
    }
    Some(BracketLink {
        label: &text[start + 1..label_end],
        dest: text[dest_start..j].trim(),
        end: j + 1,
    })
}

fn wiki_at(text: &str, i: usize) -> Option<(Link, usize)> {
    let rest = &text[i + 2..];
    let close = rest.find("]]")?;
    let inner = &rest[..close];
    if inner.contains('\n') {
        return None;
    }
    let end = i + 2 + close + 2;
    parse_wiki(inner, i, end).map(|l| (l, end))
}

fn wiki_label(link: &Link) -> String {
    if let Some(l) = &link.label {
        return l.clone();
    }
    match &link.fragment {
        Fragment::None => link.target.clone(),
        Fragment::Heading(h) if link.target.is_empty() => h.clone(),
        Fragment::Heading(h) => format!("{} \u{00a7} {}", link.target, h),
        Fragment::Block(b) => format!("{} \u{00a7} block {}", link.target, b),
    }
}

fn push_internal(
    out: &mut String,
    link: &Link,
    label_markup: &str,
    resolver: &mut dyn FnMut(&Link) -> LinkTarget,
) {
    match resolver(link) {
        LinkTarget::Href(h) => {
            out.push('[');
            out.push_str(label_markup);
            out.push_str("](");
            out.push_str(&h);
            out.push(')');
        }
        LinkTarget::Unresolved => {
            out.push_str("~~");
            out.push_str(label_markup);
            out.push_str("~~");
        }
    }
}

fn push_md_link(
    out: &mut String,
    bl: &BracketLink<'_>,
    resolver: &mut dyn FnMut(&Link) -> LinkTarget,
) {
    let dest = bl.dest.split_whitespace().next().unwrap_or("");
    let label = if bl.label.trim().is_empty() { dest } else { bl.label.trim() };
    if dest.is_empty() {
        out.push_str(&escape_lt(label));
    } else if is_http_url(dest) {
        out.push('[');
        out.push_str(&escape_lt(label));
        out.push_str("](");
        out.push_str(dest);
        out.push(')');
    } else if is_external(dest) {
        // javascript:, file:, mailto:, … are never links.
        out.push_str(&escape_lt(label));
    } else {
        let (path, fragment) = split_fragment(dest);
        if is_note_path(path) {
            let link = Link {
                kind: LinkKind::Markdown,
                target: path.to_string(),
                fragment,
                label: Some(label.to_string()),
                span: Span::new(0, 0),
            };
            push_internal(out, &link, &escape_lt(label), resolver);
        } else {
            out.push_str(&escape_lt(label));
        }
    }
}

fn find_closing_run(b: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        if b[i] == b'`' {
            let n = b[i..].iter().take_while(|&&x| x == b'`').count();
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

/// Backslash-escapes Markdown punctuation so `s` is shown literally.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '!' | '(' | ')') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escapes only `<` (keeps emphasis etc. in link labels working).
fn escape_lt(s: &str) -> String {
    s.replace('<', "\\<")
}

/// Turns `- [ ] x` / `- [x] x` list items into `- ☐ x` / `- ☑ x`.
fn task_markers(text: &str) -> String {
    text.split('\n')
        .map(task_line)
        .collect::<Vec<String>>()
        .join("\n")
}

fn task_line(line: &str) -> String {
    let t = line.trim_start();
    let indent = &line[..line.len() - t.len()];
    let tb = t.as_bytes();
    let marker_len = if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        2
    } else {
        let digits = tb.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0
            && matches!(tb.get(digits), Some(&b'.') | Some(&b')'))
            && tb.get(digits + 1) == Some(&b' ')
        {
            digits + 2
        } else {
            return line.to_string();
        }
    };
    let (marker, rest) = t.split_at(marker_len);
    let replaced = if let Some(r) = rest.strip_prefix("[ ] ") {
        format!("\u{2610} {}", r)
    } else if let Some(r) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
        format!("\u{2611} {}", r)
    } else {
        return line.to_string();
    };
    format!("{}{}{}", indent, marker, replaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(text: &str) -> String {
        display_markdown(text, &mut |_| LinkTarget::Unresolved)
    }

    fn show_resolved(text: &str) -> String {
        let mut n = 0;
        display_markdown(text, &mut |_| {
            n += 1;
            LinkTarget::Href(format!("lmf:{}", n - 1))
        })
    }

    #[test]
    fn plain_formatting_passes_through() {
        let t = "Some **bold**, *italic*, ~~gone~~ and `code` text.\n\n- a\n- b\n\n1. x\n2. y";
        assert_eq!(show(t), t);
    }

    #[test]
    fn html_is_neutralised() {
        assert_eq!(
            show("<script>alert(1)</script> and <font color=red>x</font>"),
            "\\<script>alert(1)\\</script> and \\<font color=red>x\\</font>"
        );
    }

    #[test]
    fn code_spans_and_escapes_are_untouched() {
        assert_eq!(show("use `<b>` and \\<i> here"), "use `<b>` and \\<i> here");
        assert_eq!(show("``a ` <b> ``"), "``a ` <b> ``");
    }

    #[test]
    fn wikilinks_resolved_or_struck_through() {
        assert_eq!(show_resolved("see [[Plan]] now"), "see [Plan](lmf:0) now");
        assert_eq!(show("see [[Plan]] now"), "see ~~Plan~~ now");
        assert_eq!(show_resolved("[[Plan|the plan]]"), "[the plan](lmf:0)");
        assert_eq!(show_resolved("[[Plan#Goals]]"), "[Plan § Goals](lmf:0)");
    }

    #[test]
    fn markdown_links_by_kind() {
        assert_eq!(
            show("[site](https://example.org/a?b=1)"),
            "[site](https://example.org/a?b=1)"
        );
        assert_eq!(show_resolved("[n](notes/a.md)"), "[n](lmf:0)");
        assert_eq!(show("[n](notes/a.md)"), "~~n~~");
        // Dangerous or unsupported schemes become plain text.
        assert_eq!(show("[x](javascript:alert(1))"), "x");
        assert_eq!(show("[x](file:///etc/passwd)"), "x");
        assert_eq!(show("[mail](mailto:a@b.c)"), "mail");
        // A non-note relative file is plain text (attachments are handled elsewhere).
        assert_eq!(show("[doc](files/a.pdf)"), "doc");
    }

    #[test]
    fn autolinks_and_images() {
        assert_eq!(show("<https://x.org>"), "[https://x.org](https://x.org)");
        assert_eq!(show("<javascript:x>"), "\\<javascript:x>");
        assert_eq!(show("![A cat](cat.png)"), "*(image: A cat)*");
        assert_eq!(show("![](cat.png)"), "*(image: cat.png)*");
    }

    #[test]
    fn task_list_markers() {
        assert_eq!(show("- [ ] open\n- [x] done\n  * [X] nested"), "- \u{2610} open\n- \u{2611} done\n  * \u{2611} nested");
        assert_eq!(show("1. [ ] first"), "1. \u{2610} first");
        assert_eq!(show("- [link](https://x.org)"), "- [link](https://x.org)");
    }

    #[test]
    fn stray_brackets_are_escaped_and_unicode_is_safe() {
        assert_eq!(show("a [b c"), "a \\[b c");
        assert_eq!(show("日本語 [[é]] 😀"), "日本語 ~~é~~ 😀");
        assert_eq!(show("[[unclosed"), "\\[\\[unclosed");
    }

    #[test]
    fn labels_with_markup_survive() {
        assert_eq!(
            show("[**bold** link](https://x.org)"),
            "[**bold** link](https://x.org)"
        );
    }

    #[test]
    fn hrefs_round_trip() {
        for path in ["a.md", "sub dir/My Note (v2).md", "日本語/é.md", "100%/x#y.md"] {
            let href = encode_href(path);
            assert!(href.starts_with("lmf:"));
            assert!(!href.contains(' ') && !href.contains('(') && !href.contains(')'));
            assert_eq!(decode_href(&href).as_deref(), Some(path), "{}", href);
        }
    }

    #[test]
    fn malformed_hrefs_are_rejected() {
        assert_eq!(decode_href("https://x.org"), None);
        assert_eq!(decode_href("lmf:%zz"), None);
        assert_eq!(decode_href("lmf:%4"), None);
        assert_eq!(decode_href("lmf:%FF"), None);
    }

    #[test]
    fn never_panics_on_odd_input() {
        for t in [
            "", "[", "[[", "]]", "![", "![]()", "[]()", "[a](", "[a](b", "<", "<>", "`", "``", "\\",
            "[[]]", "[[a|", "x](y)", "[a]\n(b)", "(((", "[a]((b)",
        ] {
            let _ = show(t);
            let _ = show_resolved(t);
        }
    }
}
