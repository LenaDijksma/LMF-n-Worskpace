# ADR-013: Block IDs

**Status:** proposed

**Context.** Spec §4.6/§4.7 need a way to point at a block (`[[Plan#block:milestone-03]]`, includes)
and ask how a block ID looks in plain Markdown export.

**Decision.** Obsidian-style markers kept in the text: `^id` at the end of a line labels that
paragraph/heading; `^id` alone on a line labels the block above (needed for tables and lists).
Syntax: `^` + 2 or more of `[A-Za-z0-9_-]` starting with an alphanumeric, preceded by whitespace or
at line start. IDs are unique per document and share a namespace with directive `id` attributes
(`LMF005`). Markers inside fenced code are ignored.

**Plain Markdown export.** Markers are left in place. They are readable, harmless and keep links
valid if the file is re-imported. (A future export option may strip them.)

**Rejected.** Hidden comments (`<!-- id: x -->`: invisible but easy to delete by accident and noisy);
generated numeric IDs (not stable under edits); front-matter ID tables (drift from the text).

**Known edge cases.** IDs need 2+ characters, so `x^2`, `2 ^ 3` and `text ^2` are not IDs, but a
prose fragment such as `a ^10` at the end of a line would be taken as an ID. No automatic ID
generation or insertion UI yet; IDs are written by hand.

**Open.** Block IDs for list items and table rows individually; ID assignment tooling.
