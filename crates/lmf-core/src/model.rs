//! The LMF document model (Phase 1).
//!
//! Phase 1 models the *LMF layer*: front matter, extension directives (nested, with typed
//! attributes) and the Markdown regions between them. Markdown regions are opaque
//! [`MarkdownBlock`]s that carry a source span; the Markdown baseline engine (see
//! `docs/adr/ADR-001-markdown-baseline.md`) plugs in later without changing this model.

use crate::attrs::Attr;
use crate::diagnostics::Diagnostic;
use crate::frontmatter::FrontMatter;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The exact input text. [`Document::to_source`] returns this unchanged.
    pub source: String,
    pub front_matter: Option<FrontMatter>,
    pub blocks: Vec<Block>,
    /// Syntax-level diagnostics produced while parsing. Use [`crate::check`] to also run the
    /// semantic validator.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Markdown(MarkdownBlock),
    Directive(Directive),
}

/// A run of ordinary Markdown, trimmed of leading and trailing blank lines.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownBlock {
    pub span: Span,
}

/// An extension block: `:::namespace:name {attrs}` ... `:::`.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub namespace: String,
    pub name: String,
    /// Length of the opening colon run (a closer needs at least this many).
    pub colons: usize,
    pub attrs: Vec<Attr>,
    /// False if the attribute block had a syntax error (diagnosed by the parser).
    pub attrs_valid: bool,
    pub body: Vec<Block>,
    /// False if the end of the enclosing region was reached without a closing delimiter.
    pub closed: bool,
    /// Whole directive, from the opening line through the closing line.
    pub span: Span,
    pub open_span: Span,
    pub close_span: Option<Span>,
}

impl Directive {
    pub fn qualified_name(&self) -> String {
        format!("{}:{}", self.namespace, self.name)
    }

    pub fn attr(&self, key: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.key == key)
    }

    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attr(key).and_then(|a| a.value.as_str())
    }

    pub fn attr_bool(&self, key: &str) -> Option<bool> {
        self.attr(key).and_then(|a| a.value.as_bool())
    }
}

impl Block {
    pub fn span(&self) -> Span {
        match self {
            Block::Markdown(m) => m.span,
            Block::Directive(d) => d.span,
        }
    }
}

impl Document {
    /// The original source text, byte for byte.
    pub fn to_source(&self) -> &str {
        &self.source
    }

    /// The source text covered by `span`.
    pub fn text(&self, span: Span) -> &str {
        span.slice(&self.source)
    }

    /// All directives in document order, including nested ones.
    pub fn directives(&self) -> Vec<&Directive> {
        let mut out = Vec::new();
        collect_directives(&self.blocks, &mut out);
        out
    }
}

fn collect_directives<'a>(blocks: &'a [Block], out: &mut Vec<&'a Directive>) {
    for block in blocks {
        if let Block::Directive(d) = block {
            out.push(d);
            collect_directives(&d.body, out);
        }
    }
}
