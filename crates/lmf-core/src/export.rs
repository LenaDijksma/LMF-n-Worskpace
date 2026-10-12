//! Export LMF to plain Markdown, reporting everything that could not be preserved.
//!
//! Conversion is deliberately honest: every construct that is flattened or removed produces a
//! [`Loss`] entry, so tools can show the user what a plain-Markdown copy no longer contains.
//! Line endings in the output are normalized to `\n`.

use crate::model::{Block, Directive, Document};
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownMode {
    /// Emit the unknown directive's source unchanged (visible as literal `:::` text).
    Keep,
    /// Remove it. Destructive; always reported as a [`LossLevel::Dropped`] loss.
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontMatterMode {
    Keep,
    Strip,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub unknown: UnknownMode,
    pub front_matter: FrontMatterMode,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            unknown: UnknownMode::Keep,
            front_matter: FrontMatterMode::Keep,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossLevel {
    /// Content kept; worth knowing.
    Note,
    /// Content kept, but semantics or interactivity are reduced.
    Degraded,
    /// Content removed.
    Dropped,
}

impl LossLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LossLevel::Note => "note",
            LossLevel::Degraded => "degraded",
            LossLevel::Dropped => "dropped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loss {
    pub level: LossLevel,
    pub message: String,
    /// Location in the *input* document.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub output: String,
    pub losses: Vec<Loss>,
}

/// Converts a parsed document to plain Markdown.
pub fn to_markdown(doc: &Document, opts: &ExportOptions) -> Conversion {
    let mut losses = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    if let Some(fm) = &doc.front_matter {
        match opts.front_matter {
            FrontMatterMode::Keep => {
                parts.push(normalize(doc.text(fm.span)));
                losses.push(Loss {
                    level: LossLevel::Note,
                    message: "front matter kept verbatim; many Markdown tools ignore or display it"
                        .to_string(),
                    span: fm.span,
                });
            }
            FrontMatterMode::Strip => losses.push(Loss {
                level: LossLevel::Dropped,
                message: "front matter (document metadata) removed".to_string(),
                span: fm.span,
            }),
        }
    }

    parts.push(render_blocks(doc, &doc.blocks, opts, &mut losses));
    let mut output = join(parts);
    if !output.is_empty() {
        output.push('\n');
    }
    Conversion { output, losses }
}

fn render_blocks(
    doc: &Document,
    blocks: &[Block],
    opts: &ExportOptions,
    losses: &mut Vec<Loss>,
) -> String {
    let parts: Vec<String> = blocks
        .iter()
        .map(|b| match b {
            Block::Markdown(m) => normalize(doc.text(m.span)),
            Block::Directive(d) => render_directive(doc, d, opts, losses),
        })
        .collect();
    join(parts)
}

fn render_directive(
    doc: &Document,
    dir: &Directive,
    opts: &ExportOptions,
    losses: &mut Vec<Loss>,
) -> String {
    match (dir.namespace.as_str(), dir.name.as_str()) {
        ("lmf", "callout") => {
            let kind = dir.attr_str("kind").unwrap_or("note");
            let label = capitalize(kind);
            let header = match dir.attr_str("title") {
                Some(t) if !t.is_empty() => format!("**{}: {}**", label, t),
                _ => format!("**{}**", label),
            };
            let body = render_blocks(doc, &dir.body, opts, losses);
            let mut out = format!("> {}", header);
            if !body.is_empty() {
                out.push_str("\n>\n");
                out.push_str(&quote_block(&body));
            }
            losses.push(Loss {
                level: LossLevel::Degraded,
                message: format!(
                    "`lmf:callout` ({}) converted to a blockquote; the callout type is only conveyed by its bold label",
                    kind
                ),
                span: dir.span,
            });
            out
        }
        ("lmf", "details") => {
            losses.push(Loss {
                level: LossLevel::Degraded,
                message: "`lmf:details` flattened to a bold title followed by its content; it is no longer collapsible"
                    .to_string(),
                span: dir.span,
            });
            render_titled(doc, dir, "Details", opts, losses)
        }
        ("lmf", "tabs") => {
            losses.push(Loss {
                level: LossLevel::Degraded,
                message: "`lmf:tabs` flattened into sequential titled sections; tab switching is lost"
                    .to_string(),
                span: dir.span,
            });
            let parts: Vec<String> = dir
                .body
                .iter()
                .map(|child| match child {
                    Block::Directive(d) if d.namespace == "lmf" && d.name == "tab" => {
                        render_titled(doc, d, "Tab", opts, losses)
                    }
                    other => render_blocks(doc, std::slice::from_ref(other), opts, losses),
                })
                .collect();
            join(parts)
        }
        ("lmf", "tab") => {
            losses.push(Loss {
                level: LossLevel::Degraded,
                message: "`lmf:tab` outside `lmf:tabs` flattened to a bold title followed by its content"
                    .to_string(),
                span: dir.span,
            });
            render_titled(doc, dir, "Tab", opts, losses)
        }
        _ => {
            let name = dir.qualified_name();
            match opts.unknown {
                UnknownMode::Keep => {
                    losses.push(Loss {
                        level: LossLevel::Degraded,
                        message: format!(
                            "unknown extension `{}` kept as literal directive text; plain Markdown renderers will not interpret it",
                            name
                        ),
                        span: dir.span,
                    });
                    normalize(doc.text(dir.span))
                }
                UnknownMode::Drop => {
                    losses.push(Loss {
                        level: LossLevel::Dropped,
                        message: format!("unknown extension `{}` removed", name),
                        span: dir.span,
                    });
                    String::new()
                }
            }
        }
    }
}

fn render_titled(
    doc: &Document,
    dir: &Directive,
    default_title: &str,
    opts: &ExportOptions,
    losses: &mut Vec<Loss>,
) -> String {
    let title = match dir.attr_str("title") {
        Some(t) if !t.is_empty() => t,
        _ => default_title,
    };
    let body = render_blocks(doc, &dir.body, opts, losses);
    if body.is_empty() {
        format!("**{}**", title)
    } else {
        format!("**{}**\n\n{}", title, body)
    }
}

fn quote_block(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.is_empty() {
                ">".to_string()
            } else {
                format!("> {}", l)
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn join(parts: Vec<String>) -> String {
    parts
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .collect::<Vec<String>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn export(src: &str, opts: &ExportOptions) -> Conversion {
        to_markdown(&parse(src), opts)
    }

    #[test]
    fn callout_matches_spec_example() {
        let src = ":::lmf:callout {kind=\"warning\" title=\"Before you continue\"}\nThis is a warning with **Markdown content** inside it.\n:::\n";
        let c = export(src, &ExportOptions::default());
        assert_eq!(
            c.output,
            "> **Warning: Before you continue**\n>\n> This is a warning with **Markdown content** inside it.\n"
        );
        assert_eq!(c.losses.len(), 1);
        assert_eq!(c.losses[0].level, LossLevel::Degraded);
    }

    #[test]
    fn plain_markdown_is_unchanged_and_lossless() {
        let src = "# Title\n\nText with `code`.\n\n```rust\nfn main() {}\n```\n";
        let c = export(src, &ExportOptions::default());
        assert_eq!(c.output, src);
        assert!(c.losses.is_empty());
    }

    #[test]
    fn tabs_flatten() {
        let src = ":::lmf:tabs\n:::lmf:tab {title=\"Linux\"}\nL\n:::\n:::lmf:tab {title=\"Windows\"}\nW\n:::\n:::\n";
        let c = export(src, &ExportOptions::default());
        assert_eq!(c.output, "**Linux**\n\nL\n\n**Windows**\n\nW\n");
    }

    #[test]
    fn details_flatten() {
        let c = export(":::lmf:details {title=\"More\"}\nbody\n:::\n", &ExportOptions::default());
        assert_eq!(c.output, "**More**\n\nbody\n");
    }

    #[test]
    fn unknown_keep_and_drop() {
        let src = "a\n\n:::vendor:x\npayload\n:::\n\nb\n";
        let kept = export(src, &ExportOptions::default());
        assert_eq!(kept.output, "a\n\n:::vendor:x\npayload\n:::\n\nb\n");
        assert_eq!(kept.losses[0].level, LossLevel::Degraded);

        let dropped = export(
            src,
            &ExportOptions {
                unknown: UnknownMode::Drop,
                front_matter: FrontMatterMode::Keep,
            },
        );
        assert_eq!(dropped.output, "a\n\nb\n");
        assert_eq!(dropped.losses[0].level, LossLevel::Dropped);
    }

    #[test]
    fn front_matter_modes() {
        let src = "---\ntitle: T\n---\n\nbody\n";
        let kept = export(src, &ExportOptions::default());
        assert_eq!(kept.output, "---\ntitle: T\n---\n\nbody\n");
        let stripped = export(
            src,
            &ExportOptions {
                unknown: UnknownMode::Keep,
                front_matter: FrontMatterMode::Strip,
            },
        );
        assert_eq!(stripped.output, "body\n");
        assert_eq!(stripped.losses[0].level, LossLevel::Dropped);
    }

    #[test]
    fn nested_callout_in_details() {
        let src = ":::lmf:details {title=\"D\"}\n:::lmf:callout {kind=\"tip\"}\nhi\n:::\n:::\n";
        let c = export(src, &ExportOptions::default());
        assert_eq!(c.output, "**D**\n\n> **Tip**\n>\n> hi\n");
        assert_eq!(c.losses.len(), 2);
    }

    #[test]
    fn crlf_is_normalized() {
        let c = export("a\r\nb\r\n", &ExportOptions::default());
        assert_eq!(c.output, "a\nb\n");
    }
}
