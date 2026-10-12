# ADR-002: Directive grammar

**Status:** proposed

**Context.** Spec §2.10 lists the questions a grammar must answer: delimiters, nesting, escaping,
attributes, code fences, recovery, unknown blocks.

**Decision.** Fenced blocks `:::ns:name {attrs}` … `:::` (see `docs/lmf-0.1-implemented-spec.md` §3).
Namespaced lowercase names; bare closer; closer needs ≥ opener's colons (longer outer fence is the
escape mechanism); code fences shield delimiters; attributes are typed and on one line; block-only
(no inline directives yet).

**Rejected.** Regex-based matching (breaks on nesting); `:::name` without namespace (collides with
Docusaurus/pandoc conventions and gives no ownership boundary); multi-line attribute blocks
(harder error recovery) — revisit if real documents need them.

**Consequences.** Spec example `lmf:tabs` containing `lmf:tab` at equal colon length parses
without special cases. Docusaurus-style `:::note` is left as Markdown and warned about.

**Open.** Inline directives; multi-line strings in attributes; directives inside list items.
