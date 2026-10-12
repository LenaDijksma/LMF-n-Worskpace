//! Byte ranges and line/column lookup.

/// A half-open byte range `start..end` into the original source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// The text covered by this span, or `""` if the range is not valid for `src`.
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        src.get(self.start..self.end).unwrap_or("")
    }
}

/// Maps byte offsets to 1-based `(line, column)` positions. Columns count `char`s.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { starts }
    }

    /// 1-based `(line, column)` for a byte offset (clamped to the end of `src`).
    pub fn line_col(&self, src: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(src.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.starts[line];
        let col = match src.get(line_start..offset) {
            Some(s) => s.chars().count(),
            None => offset - line_start,
        };
        (line + 1, col + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_basics() {
        let src = "ab\ncd\n";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_col(src, 0), (1, 1));
        assert_eq!(idx.line_col(src, 1), (1, 2));
        assert_eq!(idx.line_col(src, 3), (2, 1));
        assert_eq!(idx.line_col(src, 99), (3, 1));
    }

    #[test]
    fn columns_count_chars() {
        let src = "é:x";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_col(src, 2), (1, 2));
    }

    #[test]
    fn slice_is_safe() {
        assert_eq!(Span::new(1, 3).slice("abcd"), "bc");
        assert_eq!(Span::new(1, 99).slice("abcd"), "");
    }
}
