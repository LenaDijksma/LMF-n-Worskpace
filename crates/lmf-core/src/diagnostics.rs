//! Diagnostics: recoverable problems found while parsing or validating.

use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Informational; nothing is wrong (e.g. an unsupported-but-preserved extension).
    Info,
    /// Probably a mistake, but the document is still usable.
    Warning,
    /// The document violates the specification.
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable diagnostic codes. Codes are part of the public contract: never renumber.
pub mod codes {
    pub const INVALID_UTF8: &str = "LMF001";
    pub const UNSUPPORTED_VERSION: &str = "LMF002";
    pub const MALFORMED_METADATA: &str = "LMF003";
    pub const DUPLICATE_METADATA_KEY: &str = "LMF004";
    pub const DUPLICATE_ID: &str = "LMF005";
    pub const UNKNOWN_EXTENSION: &str = "LMF006";
    pub const MALFORMED_DIRECTIVE: &str = "LMF007";
    pub const INVALID_ATTRIBUTE_SYNTAX: &str = "LMF008";
    pub const INVALID_ATTRIBUTE_VALUE: &str = "LMF009";
    pub const DUPLICATE_ATTRIBUTE: &str = "LMF010";
    pub const UNKNOWN_ATTRIBUTE: &str = "LMF011";
    pub const INVALID_NESTING: &str = "LMF012";
    pub const EMPTY_BODY: &str = "LMF013";
    pub const MISSING_ATTRIBUTE: &str = "LMF014";
    pub const UNCLOSED_FENCE: &str = "LMF015";
    pub const EXTENSION_VERSION: &str = "LMF016";
    pub const INVALID_METADATA_VALUE: &str = "LMF017";
    /// A referenced image or attachment does not exist.
    pub const MISSING_ASSET: &str = "LMF018";
    /// A referenced image or attachment is rejected by policy (outside the workspace, too large,
    /// unsupported type or address).
    pub const REJECTED_ASSET: &str = "LMF019";
    /// A remote (web) image: never loaded.
    pub const REMOTE_IMAGE: &str = "LMF020";
    /// A task field (`[due: …]`, `[priority: …]`, …) has an invalid or repeated value.
    pub const INVALID_TASK_FIELD: &str = "LMF021";
    /// The same task id is used by tasks in different notes.
    pub const DUPLICATE_TASK_ID: &str = "LMF022";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        severity: Severity,
        message: impl Into<String>,
        span: Span,
    ) -> Self {
        Diagnostic {
            code,
            severity,
            message: message.into(),
            span,
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self::new(code, Severity::Error, message, span)
    }

    pub fn warning(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self::new(code, Severity::Warning, message, span)
    }

    pub fn info(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self::new(code, Severity::Info, message, span)
    }
}

/// Orders diagnostics by position, most severe first at the same position.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by_key(|d| (d.span.start, std::cmp::Reverse(d.severity)));
}
