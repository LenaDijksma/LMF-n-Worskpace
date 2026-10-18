//! Semantic validation. The parser reports *syntax* problems; the validator checks metadata
//! values, extension schemas, nesting rules and identifier uniqueness.
//!
//! Like parsing, validation is pure: it performs no I/O and runs no code.

use std::collections::HashMap;

use crate::attrs::AttrValue;
use crate::diagnostics::{codes, sort_diagnostics, Diagnostic};
use crate::frontmatter::{FrontMatter, MetaEntry, MetaValue};
use crate::model::{Block, Directive, Document};
use crate::registry::{self, AttrType, BodyRule, ExtensionSpec, RESERVED_NAMESPACE};
use crate::span::Span;

/// Highest `lmf_version` this implementation can interpret.
pub const SUPPORTED_FORMAT_VERSION: i64 = 1;

/// Semantic diagnostics only.
pub fn validate(doc: &Document) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if let Some(fm) = &doc.front_matter {
        validate_metadata(fm, &mut out);
    }
    let mut ids: HashMap<String, Span> = HashMap::new();
    validate_blocks(&doc.blocks, None, &mut ids, &mut out);

    // Invalid or repeated task fields (`[due: …]`, `[priority: …]`, …).
    out.extend(crate::tasks::parse_tasks(doc).diagnostics);

    // Block IDs (`^id`) share one namespace with directive `id` attributes.
    for b in crate::structure::block_ids(doc) {
        match ids.get(&b.id).copied() {
            Some(first) => out.push(Diagnostic::error(
                codes::DUPLICATE_ID,
                format!("duplicate id `{}`; first used at byte offset {}", b.id, first.start),
                b.marker,
            )),
            None => {
                ids.insert(b.id.clone(), b.marker);
            }
        }
    }
    sort_diagnostics(&mut out);
    out
}

/// Parser diagnostics plus semantic diagnostics, sorted by position.
pub fn check(doc: &Document) -> Vec<Diagnostic> {
    let mut all = doc.diagnostics.clone();
    all.extend(validate(doc));
    sort_diagnostics(&mut all);
    all
}

// --- metadata ---------------------------------------------------------------------------

fn validate_metadata(fm: &FrontMatter, out: &mut Vec<Diagnostic>) {
    for entry in &fm.entries {
        match entry.key.as_str() {
            "lmf_version" => match &entry.value {
                MetaValue::Int(n) if *n > SUPPORTED_FORMAT_VERSION => {
                    out.push(Diagnostic::error(
                        codes::UNSUPPORTED_VERSION,
                        format!(
                            "document requires LMF format version {}, but this implementation supports up to version {}",
                            n, SUPPORTED_FORMAT_VERSION
                        ),
                        entry.span,
                    ));
                }
                MetaValue::Int(n) if *n >= 1 => {}
                _ => out.push(Diagnostic::error(
                    codes::INVALID_METADATA_VALUE,
                    "`lmf_version` must be a positive integer",
                    entry.span,
                )),
            },
            "title" | "id" | "author" | "description" | "license" => {
                expect_string(entry, out);
            }
            "language" => {
                if let Some(s) = expect_string(entry, out) {
                    if !is_language_tag(s) {
                        out.push(Diagnostic::warning(
                            codes::INVALID_METADATA_VALUE,
                            format!("`language` value `{}` does not look like a BCP 47 language tag", s),
                            entry.span,
                        ));
                    }
                }
            }
            "created" | "modified" => {
                if let Some(s) = expect_string(entry, out) {
                    if !looks_like_date(s) {
                        out.push(Diagnostic::warning(
                            codes::INVALID_METADATA_VALUE,
                            format!("`{}` value `{}` is not an ISO 8601 date or timestamp", entry.key, s),
                            entry.span,
                        ));
                    }
                }
            }
            "tags" | "aliases" | "extensions" => expect_string_list(entry, out),
            _ => {}
        }
    }
}

fn expect_string<'a>(entry: &'a MetaEntry, out: &mut Vec<Diagnostic>) -> Option<&'a str> {
    match &entry.value {
        MetaValue::Str(s) if !s.trim().is_empty() => Some(s.as_str()),
        MetaValue::Str(_) => {
            out.push(Diagnostic::error(
                codes::INVALID_METADATA_VALUE,
                format!("`{}` must not be empty", entry.key),
                entry.span,
            ));
            None
        }
        other => {
            out.push(Diagnostic::error(
                codes::INVALID_METADATA_VALUE,
                format!(
                    "`{}` must be a string, found {}; quote the value if it is meant as text",
                    entry.key,
                    other.type_name()
                ),
                entry.span,
            ));
            None
        }
    }
}

fn expect_string_list(entry: &MetaEntry, out: &mut Vec<Diagnostic>) {
    let ok = match &entry.value {
        MetaValue::Null => true,
        MetaValue::List(items) => items.iter().all(|i| matches!(i, MetaValue::Str(_))),
        _ => false,
    };
    if !ok {
        out.push(Diagnostic::error(
            codes::INVALID_METADATA_VALUE,
            format!("`{}` must be a list of strings", entry.key),
            entry.span,
        ));
    }
}

fn is_language_tag(s: &str) -> bool {
    let mut parts = s.split('-');
    match parts.next() {
        Some(p) if (2..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphabetic()) => {}
        _ => return false,
    }
    parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn looks_like_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

// --- directives -------------------------------------------------------------------------

fn validate_blocks(
    blocks: &[Block],
    parent: Option<&str>,
    ids: &mut HashMap<String, Span>,
    out: &mut Vec<Diagnostic>,
) {
    for block in blocks {
        if let Block::Directive(dir) = block {
            validate_directive(dir, parent, ids, out);
        }
    }
}

fn validate_directive(
    dir: &Directive,
    parent: Option<&str>,
    ids: &mut HashMap<String, Span>,
    out: &mut Vec<Diagnostic>,
) {
    let qname = dir.qualified_name();
    check_common_attributes(dir, ids, out);

    match registry::lookup(&dir.namespace, &dir.name) {
        Some(spec) => check_known(dir, spec, &qname, parent, out),
        None => {
            if dir.namespace == RESERVED_NAMESPACE {
                out.push(Diagnostic::warning(
                    codes::UNKNOWN_EXTENSION,
                    format!(
                        "unknown extension `{}` in the reserved `{}` namespace (typo?); its source is preserved",
                        qname, RESERVED_NAMESPACE
                    ),
                    dir.open_span,
                ));
            } else {
                out.push(Diagnostic::info(
                    codes::UNKNOWN_EXTENSION,
                    format!(
                        "extension `{}` is not supported by this implementation; its source is preserved unchanged",
                        qname
                    ),
                    dir.open_span,
                ));
            }
        }
    }

    validate_blocks(&dir.body, Some(qname.as_str()), ids, out);
}

/// `id` and `version` are accepted on every directive.
fn check_common_attributes(
    dir: &Directive,
    ids: &mut HashMap<String, Span>,
    out: &mut Vec<Diagnostic>,
) {
    for attr in &dir.attrs {
        match attr.key.as_str() {
            "id" => match &attr.value {
                AttrValue::Str(s) if is_valid_id(s) => match ids.get(s).copied() {
                    Some(first) => out.push(Diagnostic::error(
                        codes::DUPLICATE_ID,
                        format!("duplicate id `{}`; first used at byte offset {}", s, first.start),
                        attr.span,
                    )),
                    None => {
                        ids.insert(s.clone(), attr.span);
                    }
                },
                AttrValue::Str(s) => out.push(Diagnostic::error(
                    codes::INVALID_ATTRIBUTE_VALUE,
                    format!(
                        "`id` value `{}` is not valid; use letters, digits, `_`, `-`, `.` or `:`",
                        s
                    ),
                    attr.span,
                )),
                other => out.push(Diagnostic::error(
                    codes::INVALID_ATTRIBUTE_VALUE,
                    format!("`id` must be a string, found {}", other.type_name()),
                    attr.span,
                )),
            },
            // `version` is only interpreted for extensions this implementation knows;
            // for unknown extensions the attribute is opaque and is not judged.
            "version" if registry::lookup(&dir.namespace, &dir.name).is_some() => match &attr.value {
                AttrValue::Int(n) if *n >= 1 => {
                    if let Some(spec) = registry::lookup(&dir.namespace, &dir.name) {
                        if *n > i64::from(spec.version) {
                            out.push(Diagnostic::warning(
                                codes::EXTENSION_VERSION,
                                format!(
                                    "`{}` payload version {} is newer than the supported version {}",
                                    dir.qualified_name(),
                                    n,
                                    spec.version
                                ),
                                attr.span,
                            ));
                        }
                    }
                }
                AttrValue::Int(_) => out.push(Diagnostic::error(
                    codes::INVALID_ATTRIBUTE_VALUE,
                    "`version` must be a positive integer",
                    attr.span,
                )),
                other => out.push(Diagnostic::error(
                    codes::INVALID_ATTRIBUTE_VALUE,
                    format!("`version` must be an integer, found {}", other.type_name()),
                    attr.span,
                )),
            },
            _ => {}
        }
    }
}

fn check_known(
    dir: &Directive,
    spec: &ExtensionSpec,
    qname: &str,
    parent: Option<&str>,
    out: &mut Vec<Diagnostic>,
) {
    for attr in &dir.attrs {
        if attr.key == "id" || attr.key == "version" {
            continue;
        }
        match spec.attrs.iter().find(|a| a.name == attr.key) {
            None => out.push(Diagnostic::warning(
                codes::UNKNOWN_ATTRIBUTE,
                format!("unknown attribute `{}` for `{}`", attr.key, qname),
                attr.span,
            )),
            Some(aspec) => {
                if !type_matches(aspec.ty, &attr.value) {
                    out.push(Diagnostic::error(
                        codes::INVALID_ATTRIBUTE_VALUE,
                        format!(
                            "`{}` must be {}, found {}",
                            attr.key,
                            type_label(aspec.ty),
                            attr.value.type_name()
                        ),
                        attr.span,
                    ));
                } else if let (Some(allowed), AttrValue::Str(s)) = (aspec.allowed, &attr.value) {
                    if !allowed.contains(&s.as_str()) {
                        out.push(Diagnostic::error(
                            codes::INVALID_ATTRIBUTE_VALUE,
                            format!(
                                "`{}` must be one of: {}; found `{}`",
                                attr.key,
                                allowed.join(", "),
                                s
                            ),
                            attr.span,
                        ));
                    }
                }
            }
        }
    }

    for (a, b) in spec.exclusive {
        if dir.attr(a).is_some() && dir.attr(b).is_some() {
            out.push(Diagnostic::error(
                codes::INVALID_ATTRIBUTE_VALUE,
                format!("`{}` accepts either `{}` or `{}`, not both", qname, a, b),
                dir.open_span,
            ));
        }
    }

    if dir.attrs_valid {
        for aspec in spec.attrs {
            if aspec.required && !dir.attrs.iter().any(|a| a.key == aspec.name) {
                out.push(Diagnostic::error(
                    codes::MISSING_ATTRIBUTE,
                    format!("`{}` requires the `{}` attribute", qname, aspec.name),
                    dir.open_span,
                ));
            }
        }
    }

    if let Some(required) = spec.required_parent {
        if parent != Some(required) {
            out.push(Diagnostic::error(
                codes::INVALID_NESTING,
                format!("`{}` must be placed directly inside `{}`", qname, required),
                dir.open_span,
            ));
        }
    }

    if let Some(allowed) = spec.allowed_children {
        for child in &dir.body {
            match child {
                Block::Directive(c) => {
                    let cq = c.qualified_name();
                    if !allowed.contains(&cq.as_str()) {
                        out.push(Diagnostic::error(
                            codes::INVALID_NESTING,
                            format!(
                                "`{}` may only contain {}; found `{}`",
                                qname,
                                allowed.join(", "),
                                cq
                            ),
                            c.open_span,
                        ));
                    }
                }
                Block::Markdown(m) => out.push(Diagnostic::warning(
                    codes::INVALID_NESTING,
                    format!(
                        "`{}` should contain only {}; this Markdown sits outside any of them",
                        qname,
                        allowed.join(", ")
                    ),
                    m.span,
                )),
            }
        }
    }

    if spec.body == BodyRule::Required && dir.body.is_empty() {
        out.push(Diagnostic::warning(
            codes::EMPTY_BODY,
            format!("`{}` has no content", qname),
            dir.open_span,
        ));
    }
}

fn type_matches(ty: AttrType, v: &AttrValue) -> bool {
    matches!(
        (ty, v),
        (AttrType::Str, AttrValue::Str(_))
            | (AttrType::Bool, AttrValue::Bool(_))
            | (AttrType::Int, AttrValue::Int(_))
            | (AttrType::Number, AttrValue::Int(_))
            | (AttrType::Number, AttrValue::Float(_))
    )
}

fn type_label(ty: AttrType) -> &'static str {
    match ty {
        AttrType::Str => "a string",
        AttrType::Bool => "`true` or `false`",
        AttrType::Int => "an integer",
        AttrType::Number => "a number",
    }
}

fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn codes_of(src: &str) -> Vec<&'static str> {
        check(&parse(src)).iter().map(|d| d.code).collect()
    }

    #[test]
    fn clean_document_has_no_diagnostics() {
        assert!(codes_of("---\nlmf_version: 1\ntitle: T\n---\n\n:::lmf:callout {kind=\"tip\"}\nx\n:::\n").is_empty());
    }

    #[test]
    fn version_too_new() {
        assert_eq!(codes_of("---\nlmf_version: 2\n---\n"), vec!["LMF002"]);
    }

    #[test]
    fn invalid_kind_and_missing_title() {
        assert_eq!(codes_of(":::lmf:callout {kind=\"loud\"}\nx\n:::\n"), vec!["LMF009"]);
        assert_eq!(
            codes_of(":::lmf:tabs\n:::lmf:tab\nx\n:::\n:::\n"),
            vec!["LMF014"]
        );
    }

    #[test]
    fn tab_outside_tabs() {
        assert_eq!(codes_of(":::lmf:tab {title=\"a\"}\nx\n:::\n"), vec!["LMF012"]);
    }

    #[test]
    fn duplicate_ids() {
        let src = ":::lmf:callout {id=\"a\"}\nx\n:::\n\n:::lmf:callout {id=\"a\"}\ny\n:::\n";
        assert_eq!(codes_of(src), vec!["LMF005"]);
    }

    #[test]
    fn version_attribute_is_opaque_on_unknown_extensions() {
        assert_eq!(
            codes_of(":::vendor:x {version=\"3\"}\nx\n:::\n"),
            vec!["LMF006"]
        );
        assert_eq!(
            codes_of(":::lmf:callout {version=\"3\"}\nx\n:::\n"),
            vec!["LMF009"]
        );
    }

    #[test]
    fn unknown_extensions() {
        assert_eq!(codes_of(":::vendor:thing\nx\n:::\n"), vec!["LMF006"]);
        let d = check(&parse(":::lmf:calout\nx\n:::\n"));
        assert_eq!(d[0].severity, crate::Severity::Warning);
    }

    #[test]
    fn include_rules() {
        assert!(codes_of(":::lmf:include {document=\"Plan\" section=\"Goals\"}\nfallback\n:::\n").is_empty());
        assert!(codes_of(":::lmf:include {document=\"Plan\"}\n:::\n").is_empty());
        assert_eq!(codes_of(":::lmf:include {section=\"Goals\"}\n:::\n"), vec!["LMF014"]);
        assert_eq!(
            codes_of(":::lmf:include {document=\"P\" section=\"a\" block=\"b\"}\n:::\n"),
            vec!["LMF009"]
        );
    }

    #[test]
    fn math_and_diagram_rules() {
        assert!(codes_of(":::lmf:math {format=\"latex\"}\nx\n:::\n").is_empty());
        assert!(codes_of(":::lmf:math\nx\n:::\n").is_empty());
        assert_eq!(codes_of(":::lmf:math {format=\"tex\"}\nx\n:::\n"), vec!["LMF009"]);
        assert!(codes_of(":::lmf:diagram {type=\"mermaid\"}\nA-->B\n:::\n").is_empty());
        assert_eq!(codes_of(":::lmf:diagram\nA-->B\n:::\n"), vec!["LMF014"]);
        assert_eq!(codes_of(":::lmf:diagram {type=\"python\"}\nx\n:::\n"), vec!["LMF009"]);
    }

    #[test]
    fn block_ids_must_be_unique_across_blocks_and_directives() {
        assert!(codes_of("One ^aa\n\nTwo ^bb\n").is_empty());
        assert_eq!(codes_of("One ^aa\n\nTwo ^aa\n"), vec!["LMF005"]);
        assert_eq!(
            codes_of(":::lmf:callout {id=\"aa\"}\nx\n:::\n\nPara ^aa\n"),
            vec!["LMF005"]
        );
    }

    #[test]
    fn task_field_problems_are_reported() {
        assert!(codes_of("- [ ] a [due: 2026-11-03] [priority: high] ^t-aa\n").is_empty());
        assert_eq!(codes_of("- [ ] a [due: tomorrow]\n"), vec!["LMF021"]);
        assert_eq!(
            codes_of("- [ ] a ^t-aa\n- [ ] b ^t-aa\n"),
            vec!["LMF005"]
        );
    }

    #[test]
    fn dates_and_languages() {
        assert!(is_language_tag("en"));
        assert!(is_language_tag("pt-BR"));
        assert!(!is_language_tag("english"));
        assert!(looks_like_date("2026-10-10"));
        assert!(!looks_like_date("yesterday"));
    }
}
