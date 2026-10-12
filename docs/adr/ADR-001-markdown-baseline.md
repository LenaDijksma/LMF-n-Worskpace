# ADR-001: Markdown baseline

**Status:** proposed

**Context.** The spec requires choosing an exact Markdown dialect (open decision 1). Writing a
CommonMark parser by hand is a large, bug-prone task, and Phase 1's goal is the LMF layer.

**Decision.** Baseline = **CommonMark + GFM tables, strikethrough, task lists and autolinks**.
In Phase 1 the parser treats Markdown as opaque spans between LMF constructs and only tracks
what it must (fenced code, indented code, blank lines). Inline/block Markdown parsing will come
from an existing conformant CommonMark library behind a small adapter when the renderer is built
(Phase 2), so `lmf-core` can stay dependency-free until then.

**Consequences.** No Markdown is lost or altered. Directives inside lists/blockquotes are not
recognized yet. `lmf-core` makes no CommonMark-conformance claim.

**Revisit when** the Phase 2 renderer needs inline nodes, or if an adapter dependency is judged
unacceptable.
