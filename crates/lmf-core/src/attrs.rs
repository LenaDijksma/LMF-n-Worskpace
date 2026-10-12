//! Directive attribute blocks: `{key="string" flag=true count=3 ratio=1.5}`.
//!
//! Grammar (LMF 0.1 draft):
//!
//! ```text
//! attr-block = "{" *WS [ attr *( 1*WS attr ) ] *WS "}"
//! attr       = key *WS "=" *WS value
//! key        = ( ALPHA / "_" ) *( ALPHA / DIGIT / "_" / "-" / "." )
//! value      = string / boolean / integer / float
//! string     = DQUOTE *( char / escape ) DQUOTE      ; escapes: \" \\ \n \t
//! boolean    = "true" / "false"
//! ```
//!
//! Values are always typed and strings are always quoted. A whole block must fit on the
//! directive's opening line.

use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Str(String),
    Bool(bool),
    Int(i64),
    Float(f64),
}

impl AttrValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            AttrValue::Str(_) => "string",
            AttrValue::Bool(_) => "boolean",
            AttrValue::Int(_) => "integer",
            AttrValue::Float(_) => "number",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            AttrValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            AttrValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            AttrValue::Int(n) => Some(*n),
            _ => None,
        }
    }
}

impl fmt::Display for AttrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttrValue::Str(s) => write!(f, "{:?}", s),
            AttrValue::Bool(b) => write!(f, "{}", b),
            AttrValue::Int(n) => write!(f, "{}", n),
            AttrValue::Float(x) => write!(f, "{}", x),
        }
    }
}

/// One parsed attribute. `span` covers `key=value` in the original source.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub key: String,
    pub value: AttrValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrError {
    pub message: String,
    /// Absolute byte offset of the problem.
    pub offset: usize,
}

/// Result of parsing an attribute block. Parsing stops at the first syntax error but keeps
/// the attributes read so far.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrParse {
    pub attrs: Vec<Attr>,
    pub error: Option<AttrError>,
    /// Repeated keys (the first occurrence wins), with the span of the repeat.
    pub duplicates: Vec<(String, Span)>,
}

struct Cursor<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }
}

fn err(offset: usize, message: impl Into<String>) -> AttrError {
    AttrError {
        message: message.into(),
        offset,
    }
}

/// Parses `text`, which must start with `{`. `base` is the absolute byte offset of `text`
/// in the document, used to produce absolute spans.
pub fn parse_attr_block(text: &str, base: usize) -> AttrParse {
    let mut out = AttrParse {
        attrs: Vec::new(),
        error: None,
        duplicates: Vec::new(),
    };
    let mut cur = Cursor { s: text, pos: 0 };

    if cur.peek() != Some('{') {
        out.error = Some(err(base, "attributes must start with `{`"));
        return out;
    }
    cur.bump();

    loop {
        cur.skip_ws();
        match cur.peek() {
            None => {
                out.error = Some(err(base + cur.pos, "missing closing `}`"));
                return out;
            }
            Some('}') => {
                cur.bump();
                break;
            }
            Some(_) => {}
        }

        let key_start = cur.pos;
        let key = match read_key(&mut cur) {
            Some(k) => k,
            None => {
                out.error = Some(err(base + cur.pos, "expected an attribute name"));
                return out;
            }
        };
        cur.skip_ws();
        if cur.peek() != Some('=') {
            out.error = Some(err(
                base + cur.pos,
                format!("expected `=` after attribute name `{}`", key),
            ));
            return out;
        }
        cur.bump();
        cur.skip_ws();

        let value = match read_value(&mut cur) {
            Ok(v) => v,
            Err(message) => {
                out.error = Some(err(base + cur.pos, message));
                return out;
            }
        };
        let span = Span::new(base + key_start, base + cur.pos);
        if out.attrs.iter().any(|a| a.key == key) {
            out.duplicates.push((key, span));
        } else {
            out.attrs.push(Attr { key, value, span });
        }

        match cur.peek() {
            Some(' ') | Some('\t') | Some('}') => {}
            None => {
                out.error = Some(err(base + cur.pos, "missing closing `}`"));
                return out;
            }
            Some(_) => {
                out.error = Some(err(
                    base + cur.pos,
                    "expected whitespace or `}` after the attribute value",
                ));
                return out;
            }
        }
    }

    cur.skip_ws();
    if !cur.rest().is_empty() {
        out.error = Some(err(base + cur.pos, "unexpected text after `}`"));
    }
    out
}

fn read_key(cur: &mut Cursor<'_>) -> Option<String> {
    let start = cur.pos;
    match cur.peek() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            cur.bump();
        }
        _ => return None,
    }
    while let Some(c) = cur.peek() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
            cur.bump();
        } else {
            break;
        }
    }
    Some(cur.s[start..cur.pos].to_string())
}

fn read_value(cur: &mut Cursor<'_>) -> Result<AttrValue, String> {
    match cur.peek() {
        Some('"') => read_string(cur).map(AttrValue::Str),
        Some(c) if c == '-' || c.is_ascii_digit() => read_number(cur),
        Some(c) if c.is_ascii_alphabetic() => {
            let start = cur.pos;
            while let Some(c) = cur.peek() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    cur.bump();
                } else {
                    break;
                }
            }
            match &cur.s[start..cur.pos] {
                "true" => Ok(AttrValue::Bool(true)),
                "false" => Ok(AttrValue::Bool(false)),
                other => Err(format!(
                    "unquoted value `{}`; strings must be quoted (booleans are `true` or `false`)",
                    other
                )),
            }
        }
        _ => Err("expected a value: a quoted string, number, `true` or `false`".to_string()),
    }
}

fn read_string(cur: &mut Cursor<'_>) -> Result<String, String> {
    cur.bump(); // opening quote
    let mut out = String::new();
    loop {
        match cur.bump() {
            None => return Err("unterminated string".to_string()),
            Some('"') => return Ok(out),
            Some('\\') => match cur.bump() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(c) => {
                    return Err(format!(
                        "unknown escape `\\{}`; supported escapes are \\\", \\\\, \\n and \\t",
                        c
                    ))
                }
                None => return Err("unterminated string".to_string()),
            },
            Some(c) => out.push(c),
        }
    }
}

fn read_number(cur: &mut Cursor<'_>) -> Result<AttrValue, String> {
    let start = cur.pos;
    if cur.peek() == Some('-') {
        cur.bump();
    }
    let mut digits = 0usize;
    while matches!(cur.peek(), Some(c) if c.is_ascii_digit()) {
        cur.bump();
        digits += 1;
    }
    if digits == 0 {
        return Err("expected digits after `-`".to_string());
    }
    let mut is_float = false;
    if cur.peek() == Some('.') {
        cur.bump();
        let mut frac = 0usize;
        while matches!(cur.peek(), Some(c) if c.is_ascii_digit()) {
            cur.bump();
            frac += 1;
        }
        if frac == 0 {
            return Err("expected digits after `.`".to_string());
        }
        is_float = true;
    }
    let text = &cur.s[start..cur.pos];
    if is_float {
        text.parse::<f64>()
            .map(AttrValue::Float)
            .map_err(|_| format!("invalid number `{}`", text))
    } else {
        text.parse::<i64>()
            .map(AttrValue::Int)
            .map_err(|_| format!("invalid or out-of-range integer `{}`", text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_values() {
        let p = parse_attr_block(r#"{kind="warning" open=false n=3 x=-1.5}"#, 100);
        assert!(p.error.is_none(), "{:?}", p.error);
        assert_eq!(p.attrs.len(), 4);
        assert_eq!(p.attrs[0].value, AttrValue::Str("warning".into()));
        assert_eq!(p.attrs[1].value, AttrValue::Bool(false));
        assert_eq!(p.attrs[2].value, AttrValue::Int(3));
        assert_eq!(p.attrs[3].value, AttrValue::Float(-1.5));
        assert_eq!(p.attrs[0].span.start, 101);
    }

    #[test]
    fn empty_block_is_fine() {
        let p = parse_attr_block("{ }", 0);
        assert!(p.error.is_none());
        assert!(p.attrs.is_empty());
    }

    #[test]
    fn string_escapes_and_unicode() {
        let p = parse_attr_block(r#"{title="a \"q\" \\ é\n"}"#, 0);
        assert!(p.error.is_none(), "{:?}", p.error);
        assert_eq!(p.attrs[0].value, AttrValue::Str("a \"q\" \\ é\n".into()));
    }

    #[test]
    fn braces_inside_strings_are_content() {
        let p = parse_attr_block(r#"{title="a } b"}"#, 0);
        assert!(p.error.is_none());
        assert_eq!(p.attrs[0].value, AttrValue::Str("a } b".into()));
    }

    #[test]
    fn rejects_unquoted_strings() {
        let p = parse_attr_block("{kind=warning}", 0);
        assert!(p.error.is_some());
        assert!(p.attrs.is_empty());
    }

    #[test]
    fn rejects_missing_close_and_trailing_text() {
        assert!(parse_attr_block(r#"{kind="x""#, 0).error.is_some());
        assert!(parse_attr_block(r#"{kind="x"} trailing"#, 0).error.is_some());
        assert!(parse_attr_block(r#"{kind="x}"#, 0).error.is_some());
    }

    #[test]
    fn rejects_flag_without_value() {
        assert!(parse_attr_block("{open}", 0).error.is_some());
    }

    #[test]
    fn reports_duplicates_and_keeps_first() {
        let p = parse_attr_block(r#"{a=1 a=2}"#, 0);
        assert!(p.error.is_none());
        assert_eq!(p.attrs.len(), 1);
        assert_eq!(p.attrs[0].value, AttrValue::Int(1));
        assert_eq!(p.duplicates.len(), 1);
    }
}
