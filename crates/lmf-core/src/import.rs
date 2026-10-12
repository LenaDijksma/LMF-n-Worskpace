//! Import ordinary Markdown into LMF.
//!
//! Every Markdown document is already valid LMF, so import is *optional* enrichment. Phase 1
//! converts one well-defined convention and leaves everything else untouched:
//!
//! * GitHub-style alerts (`> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`)
//!   become `lmf:callout` blocks.
//!
//! Optionally a minimal `lmf_version` front matter block is added to documents that have none.
//! Line endings in the output are normalized to `\n`.

use crate::export::{Conversion, Loss, LossLevel};
use crate::lines::{fence_close, fence_open, indent_of, split_lines};
use crate::parser::parse;
use crate::span::Span;
use crate::validate::SUPPORTED_FORMAT_VERSION;

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Prepend `lmf_version` front matter if the document has no front matter.
    pub stamp_version: bool,
}

pub fn from_markdown(src: &str, opts: &ImportOptions) -> Conversion {
    let lines = split_lines(src);
    let mut out = String::new();
    let mut losses: Vec<Loss> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];

        if let Some((ch, len)) = fence {
            if fence_close(line.text, ch, len) {
                fence = None;
            }
            push_line(&mut out, line.text);
            i += 1;
            continue;
        }
        if let Some(open) = fence_open(line.text) {
            fence = Some(open);
            push_line(&mut out, line.text);
            i += 1;
            continue;
        }

        if let Some(kind) = alert_kind(line.text) {
            let mut j = i + 1;
            let mut body: Vec<&str> = Vec::new();
            while j < lines.len() {
                match strip_quote(lines[j].text) {
                    Some(rest) => {
                        body.push(rest);
                        j += 1;
                    }
                    None => break,
                }
            }
            // Use a longer fence if the body contains colon-only lines that would collide.
            let longest = body
                .iter()
                .map(|l| l.trim_start().bytes().take_while(|&b| b == b':').count())
                .max()
                .unwrap_or(0);
            let colons = ":".repeat(longest.max(2) + 1);

            out.push_str(&format!("{}lmf:callout {{kind=\"{}\"}}\n", colons, kind));
            for b in &body {
                push_line(&mut out, b);
            }
            out.push_str(&colons);
            out.push('\n');

            losses.push(Loss {
                level: LossLevel::Note,
                message: format!("GitHub-style alert converted to `lmf:callout` (kind \"{}\")", kind),
                span: Span::new(line.start, lines[j - 1].end),
            });
            i = j;
            continue;
        }

        push_line(&mut out, line.text);
        i += 1;
    }

    if opts.stamp_version {
        let doc = parse(src);
        match &doc.front_matter {
            None => {
                out = format!("---\nlmf_version: {}\n---\n\n{}", SUPPORTED_FORMAT_VERSION, out);
                losses.push(Loss {
                    level: LossLevel::Note,
                    message: format!("added front matter with `lmf_version: {}`", SUPPORTED_FORMAT_VERSION),
                    span: Span::new(0, 0),
                });
            }
            Some(fm) => {
                if fm.get("lmf_version").is_none() {
                    losses.push(Loss {
                        level: LossLevel::Note,
                        message: "front matter has no `lmf_version`; it was not modified, add the key manually"
                            .to_string(),
                        span: fm.span,
                    });
                }
            }
        }
    }

    Conversion {
        output: out,
        losses,
    }
}

fn push_line(out: &mut String, text: &str) {
    out.push_str(text);
    out.push('\n');
}

/// Strips one level of blockquote marker (`>` plus an optional space), if present.
fn strip_quote(text: &str) -> Option<&str> {
    let indent = indent_of(text);
    if indent > 3 {
        return None;
    }
    let rest = text[indent..].strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn alert_kind(text: &str) -> Option<&'static str> {
    let rest = strip_quote(text)?;
    let inner = rest.trim().strip_prefix("[!")?.strip_suffix(']')?;
    match inner.to_ascii_uppercase().as_str() {
        "NOTE" => Some("note"),
        "TIP" => Some("tip"),
        "IMPORTANT" => Some("info"),
        "WARNING" => Some("warning"),
        "CAUTION" => Some("danger"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::check;

    #[test]
    fn converts_github_alert() {
        let c = from_markdown("> [!NOTE]\n> Useful information.\n", &ImportOptions::default());
        assert_eq!(c.output, ":::lmf:callout {kind=\"note\"}\nUseful information.\n:::\n");
        assert_eq!(c.losses.len(), 1);
        assert!(check(&parse(&c.output)).is_empty());
    }

    #[test]
    fn alert_kinds_map() {
        for (marker, kind) in [
            ("TIP", "tip"),
            ("IMPORTANT", "info"),
            ("WARNING", "warning"),
            ("CAUTION", "danger"),
        ] {
            let src = format!("> [!{}]\n> x\n", marker);
            let c = from_markdown(&src, &ImportOptions::default());
            assert!(c.output.contains(&format!("kind=\"{}\"", kind)), "{}", c.output);
        }
    }

    #[test]
    fn ordinary_quotes_and_code_are_untouched() {
        let src = "> just a quote\n\n```md\n> [!NOTE]\n> inside code\n```\n";
        let c = from_markdown(src, &ImportOptions::default());
        assert_eq!(c.output, src);
        assert!(c.losses.is_empty());
    }

    #[test]
    fn text_around_alert_is_kept() {
        let src = "before\n\n> [!WARNING]\n> careful\n\nafter\n";
        let c = from_markdown(src, &ImportOptions::default());
        assert_eq!(
            c.output,
            "before\n\n:::lmf:callout {kind=\"warning\"}\ncareful\n:::\n\nafter\n"
        );
    }

    #[test]
    fn colon_lines_in_body_get_a_longer_fence() {
        let c = from_markdown("> [!TIP]\n> :::\n> x\n", &ImportOptions::default());
        assert!(c.output.starts_with("::::lmf:callout"));
        let doc = parse(&c.output);
        assert!(check(&doc).is_empty(), "{:?}", check(&doc));
    }

    #[test]
    fn stamp_adds_front_matter_once() {
        let opts = ImportOptions { stamp_version: true };
        let c = from_markdown("# Hi\n", &opts);
        assert_eq!(c.output, "---\nlmf_version: 1\n---\n\n# Hi\n");
        let again = from_markdown(&c.output, &opts);
        assert_eq!(again.output, c.output);
    }
}
