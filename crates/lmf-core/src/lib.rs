//! # lmf-core
//!
//! Reference parser, document model, validator and converters for **LMF — Lovely Markdown
//! Format** (spec draft 0.1).
//!
//! Design rules this crate follows (see `docs/`):
//!
//! * **Markdown first.** Any text is a valid LMF document. Ordinary Markdown is carried
//!   through untouched as opaque [`MarkdownBlock`]s.
//! * **Source preserving.** Every node records a byte [`Span`] into the original text and
//!   [`Document::to_source`] always returns the input exactly. Unknown or malformed extension
//!   regions are never dropped.
//! * **Parsing is not execution.** Nothing here performs I/O, network access or evaluation.
//! * **Recoverable errors.** Problems become [`Diagnostic`]s; the surrounding content stays
//!   readable.
//!
//! ```
//! let doc = lmf_core::parse(":::lmf:callout {kind=\"tip\"}\nHello\n:::\n");
//! assert!(lmf_core::check(&doc).is_empty());
//! ```

pub mod attrs;
pub mod diagnostics;
pub mod export;
pub mod frontmatter;
pub mod import;
pub mod links;
pub(crate) mod lines;
pub mod model;
pub mod parser;
pub mod preview;
pub mod registry;
pub mod richtext;
pub mod span;
pub mod structure;
pub mod validate;

pub use diagnostics::{Diagnostic, Severity};
pub use model::{Block, Directive, Document, MarkdownBlock};
pub use parser::{parse, parse_bytes};
pub use span::{LineIndex, Span};
pub use validate::{check, validate, SUPPORTED_FORMAT_VERSION};
