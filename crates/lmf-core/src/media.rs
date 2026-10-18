//! Images and attachments referenced from Markdown.
//!
//! Extraction is pure and syntactic. Whether a file exists, how big it is and whether it may be
//! shown or opened is decided by the front end / `lmf-workspace` (which touch the filesystem).
//!
//! * an **image** is `![alt](address)`;
//! * an **attachment** is an ordinary link `[text](address)` to a *local* file that is not a note
//!   (has a file extension, is not `.md`/`.lmf`/`.markdown`), e.g. `[report](assets/report.pdf)`.
//!
//! Addresses are classified as [`MediaTarget::Local`] (a relative path, percent-decoded),
//! [`MediaTarget::Remote`] (`http(s)`; never fetched) or [`MediaTarget::Unsupported`]
//! (`data:`, `file:`, other schemes, empty).
//!
//! References inside code (fenced or inline) and inside unknown extension blocks are ignored.

use crate::lines::{fence_close, fence_open, split_lines};
use crate::links::{is_http_url, is_note_path};
use crate::model::{Block, Document};
use crate::registry;
use crate::richtext::bracket_link;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaTarget {
    /// A relative path (percent-decoded, without `#fragment` / `?query`).
    Local(String),
    /// An `http(s)` URL. Never fetched.
    Remote(String),
    /// Anything else (`data:`, `file:`, `javascript:`, empty, …). Never used.
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    pub kind: MediaKind,
    pub target: MediaTarget,
    /// Alt text (images) or link text (attachments).
    pub label: String,
    /// The whole `![alt](address)` / `[text](address)` in the document source.
    pub span: Span,
}

/// All image and attachment references in a document, in source order.
pub fn extract_media(doc: &Document) -> Vec<MediaRef> {
    let mut out = Vec::new();
    collect(doc, &doc.blocks, &mut out);
    out.sort_by_key(|m| m.span.start);
    out
}

fn collect(doc: &Document, blocks: &[Block], out: &mut Vec<MediaRef>) {
    for block in blocks {
        match block {
            Block::Markdown(m) => scan_markdown(doc.text(m.span), m.span.start, out),
            Block::Directive(d) => {
                if registry::lookup(&d.namespace, &d.name).is_some() {
                    collect(doc, &d.body, out);
                }
            }
        }
    }
}

fn scan_markdown(text: &str, base: usize, out: &mut Vec<MediaRef>) {
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
        scan_line(line.text, base + line.start, out);
    }
}

fn scan_line(line: &str, line_start: usize, out: &mut Vec<MediaRef>) {
    let b = line.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'`' => {
                let run = b[i..].iter().take_while(|&&x| x == b'`').count();
                i = match find_run(b, i + run, run) {
                    Some(end) => end + run,
                    None => i + run,
                };
            }
            b'\\' => i += 2,
            b'!' if b.get(i + 1) == Some(&b'[') => match bracket_link(line, i + 1) {
                Some(bl) => {
                    out.push(MediaRef {
                        kind: MediaKind::Image,
                        target: classify_target(bl.dest),
                        label: bl.label.trim().to_string(),
                        span: Span::new(line_start + i, line_start + bl.end),
                    });
                    i = bl.end;
                }
                None => i += 1,
            },
            // `[[wikilink]]`: not media.
            b'[' if b.get(i + 1) == Some(&b'[') => i += 2,
            b'[' => match bracket_link(line, i) {
                Some(bl) => {
                    if let MediaTarget::Local(path) = classify_target(bl.dest) {
                        if !is_note_path(&path) && has_file_extension(&path) {
                            out.push(MediaRef {
                                kind: MediaKind::Attachment,
                                target: MediaTarget::Local(path),
                                label: bl.label.trim().to_string(),
                                span: Span::new(line_start + i, line_start + bl.end),
                            });
                        }
                    }
                    i = bl.end;
                }
                None => i += 1,
            },
            _ => i += 1,
        }
    }
}

fn find_run(b: &[u8], from: usize, run: usize) -> Option<usize> {
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

/// The address part of `(…)` in a link: a `<…>` form (may contain spaces) or the first word
/// (the rest is an optional title).
pub fn clean_dest(dest: &str) -> String {
    let d = dest.trim();
    if let Some(rest) = d.strip_prefix('<') {
        if let Some(end) = rest.find('>') {
            return rest[..end].to_string();
        }
    }
    d.split_whitespace().next().unwrap_or("").to_string()
}

/// Classifies a link address. Local addresses are percent-decoded and lose any `#fragment` or
/// `?query`.
pub fn classify_target(dest: &str) -> MediaTarget {
    let d = clean_dest(dest);
    if d.is_empty() {
        return MediaTarget::Unsupported(d);
    }
    if is_http_url(&d) {
        return MediaTarget::Remote(d);
    }
    let lower = d.to_ascii_lowercase();
    let blocked_scheme = ["data:", "javascript:", "mailto:", "file:", "blob:", "vbscript:"]
        .iter()
        .any(|s| lower.starts_with(s));
    if lower.contains("://") || lower.starts_with("//") || blocked_scheme {
        return MediaTarget::Unsupported(d);
    }
    let path = d.split(|c| c == '#' || c == '?').next().unwrap_or("");
    if path.is_empty() {
        return MediaTarget::Unsupported(d);
    }
    MediaTarget::Local(decode_percent(path))
}

/// Decodes `%XX` sequences. Invalid sequences or invalid UTF-8 leave the input unchanged.
pub fn decode_percent(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = hex(b[i + 1]);
            let lo = hex(b[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Encodes a path for use as a Markdown link address (spaces, parentheses and a few other
/// characters would break the link). Non-ASCII text is kept as is.
pub fn encode_dest(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, ' ' | '(' | ')' | '<' | '>' | '%' | '#' | '?' | '\\') {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{:02X}", byte));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether the last path segment has a short alphanumeric file extension (`a.pdf`, not `a/`).
pub fn has_file_extension(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// If `line` consists of exactly one image, returns `(alt, address)` (address as written, cleaned
/// of the optional title).
pub(crate) fn parse_image_line(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if !t.starts_with("![") {
        return None;
    }
    let bl = bracket_link(t, 1)?;
    if bl.end != t.len() {
        return None;
    }
    Some((bl.label.trim().to_string(), clean_dest(bl.dest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn media(src: &str) -> Vec<MediaRef> {
        extract_media(&parse(src))
    }

    #[test]
    fn images_and_attachments() {
        let m = media("![A cat](assets/cat.png) and [report](assets/report.pdf) and [n](note.md) and [w](https://x.org/a.pdf)");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].kind, MediaKind::Image);
        assert_eq!(m[0].target, MediaTarget::Local("assets/cat.png".to_string()));
        assert_eq!(m[0].label, "A cat");
        assert_eq!(m[1].kind, MediaKind::Attachment);
        assert_eq!(m[1].target, MediaTarget::Local("assets/report.pdf".to_string()));
    }

    #[test]
    fn remote_and_unsupported_images() {
        let m = media("![a](https://x.org/a.png) ![b](data:image/png;base64,AAAA) ![c](file:///etc/passwd) ![d]()");
        assert_eq!(m.len(), 4);
        assert!(matches!(m[0].target, MediaTarget::Remote(_)));
        assert!(matches!(m[1].target, MediaTarget::Unsupported(_)));
        assert!(matches!(m[2].target, MediaTarget::Unsupported(_)));
        assert!(matches!(m[3].target, MediaTarget::Unsupported(_)));
    }

    #[test]
    fn addresses_are_decoded_and_cleaned() {
        assert_eq!(
            classify_target("my%20pics/a%20b.png#frag"),
            MediaTarget::Local("my pics/a b.png".to_string())
        );
        assert_eq!(
            classify_target("<my pics/a b.png> \"title\""),
            MediaTarget::Local("my pics/a b.png".to_string())
        );
        assert_eq!(
            classify_target("a.png \"A title\""),
            MediaTarget::Local("a.png".to_string())
        );
        assert_eq!(
            classify_target("100%25.png"),
            MediaTarget::Local("100%.png".to_string())
        );
    }

    #[test]
    fn code_and_unknown_blocks_are_ignored() {
        let m = media("`![x](a.png)`\n\n```\n![y](b.png)\n```\n\n:::vendor:x\n![z](c.png)\n:::\n\n:::lmf:callout\n![ok](d.png)\n:::\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, "ok");
    }

    #[test]
    fn wikilinks_and_folders_are_not_attachments() {
        assert!(media("[[Some Note]] [dir](assets/) [x](#heading) [y]()").is_empty());
    }

    #[test]
    fn spans_cover_the_reference() {
        let src = "text ![alt](a.png) more";
        let doc = parse(src);
        let m = extract_media(&doc);
        assert_eq!(doc.text(m[0].span), "![alt](a.png)");
    }

    #[test]
    fn encode_dest_round_trips_through_classify() {
        for p in ["a b/c (1).png", "100%/x#y.png", "é/日本語.png"] {
            let enc = encode_dest(p);
            assert!(!enc.contains(' ') && !enc.contains('(') && !enc.contains(')'));
            assert_eq!(classify_target(&enc), MediaTarget::Local(p.to_string()), "{}", enc);
        }
    }

    #[test]
    fn extension_detection() {
        assert!(has_file_extension("a/b.pdf"));
        assert!(!has_file_extension("a/b"));
        assert!(!has_file_extension("a/"));
        assert!(!has_file_extension(".hidden"));
        assert!(!has_file_extension("a.toolongext"));
    }

    #[test]
    fn single_image_lines() {
        assert_eq!(
            parse_image_line("  ![A cat](cat.png \"t\")  "),
            Some(("A cat".to_string(), "cat.png".to_string()))
        );
        assert_eq!(parse_image_line("![a](b.png) trailing"), None);
        assert_eq!(parse_image_line("text ![a](b.png)"), None);
    }

    #[test]
    fn never_panics() {
        for t in ["![", "![]", "![](", "![a](b", "[a](b", "%", "![a](%)", "![a](%4)", "![a](<b)"] {
            let _ = media(t);
        }
    }
}
