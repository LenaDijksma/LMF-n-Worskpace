# ADR-003: Metadata syntax

**Status:** proposed

**Decision.** `---` delimited front matter containing a small, fully specified YAML subset
(scalars, inline lists, block lists, comments). Absent front matter is valid. Detection requires a
`key:`-shaped first line so ordinary Markdown with thematic breaks is never misread.

**Rejected.** Full YAML (large surface, vendor-dependent behavior, an external parser dependency);
TOML/JSON front matter (less familiar to Markdown users).

**Consequences.** Duplicate keys, malformed lines and bad values are all diagnosed; unknown keys are
preserved in order. Nested mappings are an explicit non-feature for 0.1.

**Revisit when** a real feature needs structured metadata (e.g. extension declarations with version
constraints).
