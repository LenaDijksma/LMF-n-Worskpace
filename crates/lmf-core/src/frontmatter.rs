//! Document metadata ("front matter").
//!
//! LMF 0.1 uses a deliberately small YAML subset, delimited by `---` lines at the very top
//! of the file (the closing line may also be `...`):
//!
//! * `key: value` scalars: strings (bare or quoted), integers, `true`/`false`, `null`
//! * inline lists: `tags: [a, "b c"]`
//! * block lists:  `tags:` followed by `- item` lines
//! * `#` comment lines and blank lines
//!
//! Nested mappings, anchors, multi-line scalars and trailing comments are **not** supported
//! and produce a diagnostic. Unknown keys are preserved in order.
//!
//! To keep ordinary Markdown ordinary, a leading `---` block is only treated as front matter
//! if its first meaningful line looks like `key: ...`. Otherwise the `---` lines are plain
//! thematic breaks.

use crate::diagnostics::{codes, Diagnostic};
use crate::lines::Line;
use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<MetaValue>),
}

impl MetaValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            MetaValue::Null => "null",
            MetaValue::Bool(_) => "boolean",
            MetaValue::Int(_) => "integer",
            MetaValue::Str(_) => "string",
            MetaValue::List(_) => "list",
        }
    }
}

impl fmt::Display for MetaValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetaValue::Null => write!(f, "null"),
            MetaValue::Bool(b) => write!(f, "{}", b),
            MetaValue::Int(n) => write!(f, "{}", n),
            MetaValue::Str(s) => write!(f, "{:?}", s),
            MetaValue::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetaEntry {
    pub key: String,
    pub value: MetaValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrontMatter {
    /// The whole block including both delimiter lines.
    pub span: Span,
    pub entries: Vec<MetaEntry>,
}

impl FrontMatter {
    /// First entry with this key.
    pub fn get(&self, key: &str) -> Option<&MetaEntry> {
        self.entries.iter().find(|e| e.key == key)
    }
}

/// Detects and parses front matter. Returns it together with the number of source lines it
/// occupies (so the caller can continue after it).
pub(crate) fn detect_front_matter(
    lines: &[Line<'_>],
    diags: &mut Vec<Diagnostic>,
) -> Option<(FrontMatter, usize)> {
    let first = lines.first()?;
    if first.text.trim_end() != "---" {
        return None;
    }
    let close = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| {
            let t = l.text.trim_end();
            t == "---" || t == "..."
        })
        .map(|(i, _)| i)?;
    let inner = &lines[1..close];
    let first_meaningful = inner.iter().find(|l| {
        let t = l.text.trim();
        !t.is_empty() && !t.starts_with('#')
    })?;
    split_key(first_meaningful.text)?;

    let entries = parse_entries(inner, diags);
    let fm = FrontMatter {
        span: Span::new(first.start, lines[close].end),
        entries,
    };
    Some((fm, close + 1))
}

/// Splits `key: rest`. The key must start at column 0 and be a simple identifier, and the
/// colon must be followed by whitespace or the end of the line.
fn split_key(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(':')?;
    let key = &line[..idx];
    let mut chars = key.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return None;
    }
    let rest = &line[idx + 1..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    Some((key, rest))
}

fn parse_entries(inner: &[Line<'_>], diags: &mut Vec<Diagnostic>) -> Vec<MetaEntry> {
    let mut entries: Vec<MetaEntry> = Vec::new();
    let mut i = 0usize;
    while i < inner.len() {
        let line = inner[i];
        let trimmed = line.text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let Some((key, rest)) = split_key(line.text) else {
            diags.push(Diagnostic::error(
                codes::MALFORMED_METADATA,
                format!(
                    "cannot parse metadata line `{}`; expected `key: value` (nested mappings are not supported in LMF 0.1 metadata)",
                    trimmed
                ),
                Span::new(line.start, line.end),
            ));
            i += 1;
            continue;
        };
        let rest = rest.trim();
        let mut end = line.end;
        let value;
        if rest.is_empty() {
            let mut items = Vec::new();
            let mut j = i + 1;
            while j < inner.len() {
                let candidate = inner[j].text.trim_start();
                if candidate == "-" {
                    items.push(MetaValue::Null);
                } else if let Some(item) = candidate.strip_prefix("- ") {
                    items.push(parse_scalar(item));
                } else {
                    break;
                }
                end = inner[j].end;
                j += 1;
            }
            value = if items.is_empty() {
                MetaValue::Null
            } else {
                MetaValue::List(items)
            };
            i = j;
        } else {
            value = parse_inline_value(rest, line, diags);
            i += 1;
        }
        let span = Span::new(line.start, end);
        if entries.iter().any(|e| e.key == key) {
            diags.push(Diagnostic::error(
                codes::DUPLICATE_METADATA_KEY,
                format!("duplicate metadata key `{}`", key),
                span,
            ));
        }
        entries.push(MetaEntry {
            key: key.to_string(),
            value,
            span,
        });
    }
    entries
}

fn parse_inline_value(rest: &str, line: Line<'_>, diags: &mut Vec<Diagnostic>) -> MetaValue {
    if rest.starts_with('[') {
        match parse_inline_list(rest) {
            Some(items) => MetaValue::List(items),
            None => {
                diags.push(Diagnostic::error(
                    codes::MALFORMED_METADATA,
                    "unterminated or malformed inline list; expected `[a, b, c]`",
                    Span::new(line.start, line.end),
                ));
                MetaValue::Str(rest.to_string())
            }
        }
    } else {
        parse_scalar(rest)
    }
}

fn parse_inline_list(s: &str) -> Option<Vec<MetaValue>> {
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    cur.push(c);
                } else if c == ',' {
                    items.push(parse_scalar(cur.trim()));
                    cur.clear();
                } else {
                    cur.push(c);
                }
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if !cur.trim().is_empty() {
        items.push(parse_scalar(cur.trim()));
    }
    Some(items)
}

fn parse_scalar(s: &str) -> MetaValue {
    let s = s.trim();
    if s.len() >= 2 {
        let double = s.starts_with('"') && s.ends_with('"');
        let single = s.starts_with('\'') && s.ends_with('\'');
        if double || single {
            return MetaValue::Str(unquote(&s[1..s.len() - 1], double));
        }
    }
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return MetaValue::Null,
        "true" | "True" | "TRUE" => return MetaValue::Bool(true),
        "false" | "False" | "FALSE" => return MetaValue::Bool(false),
        _ => {}
    }
    if let Ok(n) = s.parse::<i64>() {
        return MetaValue::Int(n);
    }
    MetaValue::Str(s.to_string())
}

fn unquote(inner: &str, double: bool) -> String {
    if double {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        inner.replace("''", "'")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::split_lines;

    fn parse(src: &str) -> (Option<(FrontMatter, usize)>, Vec<Diagnostic>) {
        let lines = split_lines(src);
        let mut diags = Vec::new();
        let r = detect_front_matter(&lines, &mut diags);
        (r, diags)
    }

    #[test]
    fn parses_spec_sketch() {
        let src = "---\nlmf_version: 1\ntitle: My project notes\nid: doc_01HXYZ\nlanguage: en\ntags:\n  - project\n  - notes\n---\n\n# Hi\n";
        let (r, diags) = parse(src);
        assert!(diags.is_empty(), "{:?}", diags);
        let (fm, consumed) = r.expect("front matter");
        assert_eq!(consumed, 9);
        assert_eq!(fm.get("lmf_version").unwrap().value, MetaValue::Int(1));
        assert_eq!(
            fm.get("tags").unwrap().value,
            MetaValue::List(vec![
                MetaValue::Str("project".into()),
                MetaValue::Str("notes".into())
            ])
        );
        assert_eq!(fm.get("title").unwrap().value, MetaValue::Str("My project notes".into()));
    }

    #[test]
    fn inline_lists_and_quotes() {
        let (r, diags) = parse("---\ntags: [a, \"b, c\", 'd']\nx: \"q\"\n---\n");
        assert!(diags.is_empty());
        let (fm, _) = r.unwrap();
        assert_eq!(
            fm.get("tags").unwrap().value,
            MetaValue::List(vec![
                MetaValue::Str("a".into()),
                MetaValue::Str("b, c".into()),
                MetaValue::Str("d".into())
            ])
        );
    }

    #[test]
    fn thematic_breaks_are_not_front_matter() {
        let (r, _) = parse("---\n\nJust text.\n\n---\n");
        assert!(r.is_none());
        let (r, _) = parse("---\n---\n");
        assert!(r.is_none());
        let (r, _) = parse("# Title\n---\nkey: v\n---\n");
        assert!(r.is_none());
    }

    #[test]
    fn unterminated_is_not_front_matter() {
        let (r, _) = parse("---\ntitle: x\n");
        assert!(r.is_none());
    }

    #[test]
    fn reports_malformed_and_duplicate() {
        let (r, diags) = parse("---\ntitle: a\ntitle: b\n  nested: c\n---\n");
        assert!(r.is_some());
        let codes: Vec<&str> = diags.iter().map(|d| d.code).collect();
        assert!(codes.contains(&"LMF004"));
        assert!(codes.contains(&"LMF003"));
    }
}
