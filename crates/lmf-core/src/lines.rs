//! Line splitting and fenced-code-block helpers shared by the parser and importer.

/// One source line. Offsets are byte offsets into the original text.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Line<'a> {
    /// Offset of the first character (after a leading BOM, if any).
    pub start: usize,
    /// Offset just past the last character, excluding `\r` and `\n`.
    pub end: usize,
    /// The line text without its terminator.
    pub text: &'a str,
}

pub(crate) fn split_lines(src: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < src.len() {
        let rest = &src[pos..];
        let (line_len, advance) = match rest.find('\n') {
            Some(i) => (i, i + 1),
            None => (rest.len(), rest.len()),
        };
        let bom = if pos == 0 && rest.starts_with('\u{FEFF}') {
            '\u{FEFF}'.len_utf8()
        } else {
            0
        };
        let raw = &rest[bom..line_len];
        let text = raw.strip_suffix('\r').unwrap_or(raw);
        out.push(Line {
            start: pos + bom,
            end: pos + bom + text.len(),
            text,
        });
        pos += advance;
    }
    out
}

pub(crate) fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// Number of leading spaces.
pub(crate) fn indent_of(text: &str) -> usize {
    text.len() - text.trim_start_matches(' ').len()
}

/// If `text` opens a fenced code block (``` or ~~~, indented at most 3 spaces), returns the
/// fence character and its length.
pub(crate) fn fence_open(text: &str) -> Option<(char, usize)> {
    let indent = indent_of(text);
    if indent > 3 {
        return None;
    }
    let body = &text[indent..];
    let ch = body.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = body.chars().take_while(|&c| c == ch).count();
    if n < 3 {
        return None;
    }
    let info = &body[n..];
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some((ch, n))
}

/// True if `text` closes a fence opened with `ch` repeated `len` times.
pub(crate) fn fence_close(text: &str, ch: char, len: usize) -> bool {
    let indent = indent_of(text);
    if indent > 3 {
        return false;
    }
    let body = &text[indent..];
    let n = body.chars().take_while(|&c| c == ch).count();
    n >= len && body[n..].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lf_crlf_and_missing_final_newline() {
        let lines = split_lines("a\r\nb\nc");
        let texts: Vec<&str> = lines.iter().map(|l| l.text).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
        assert_eq!(lines[0].end, 1);
        assert_eq!(lines[1].start, 3);
        assert_eq!(lines[2].start, 5);
    }

    #[test]
    fn bom_is_skipped() {
        let lines = split_lines("\u{FEFF}# hi\n");
        assert_eq!(lines[0].text, "# hi");
        assert_eq!(lines[0].start, 3);
    }

    #[test]
    fn fences() {
        assert_eq!(fence_open("```rust"), Some(('`', 3)));
        assert_eq!(fence_open("~~~~"), Some(('~', 4)));
        assert_eq!(fence_open("``` a`b"), None);
        assert_eq!(fence_open("    ```"), None);
        assert!(fence_close("````", '`', 3));
        assert!(!fence_close("``", '`', 3));
        assert!(!fence_close("``` x", '`', 3));
    }
}
