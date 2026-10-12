# ADR-005: Source-preserving parser strategy

**Status:** proposed

**Decision.** The document model never owns rewritten text. Nodes carry byte `Span`s into the
original source and `Document::to_source()` returns the input unchanged. Edits in future tooling
should be expressed as span replacements, so untouched regions (including unknown extensions)
survive byte for byte. Parsing is a single pass with an explicit stack; no I/O, no evaluation.

**Consequences.** Round-tripping is trivially lossless (tested). The cost is that nodes hold
offsets rather than text, so consumers go through `Document::text(span)`.
Invalid UTF-8 is decoded lossily for analysis only, and writers must refuse to save it.

**Revisit when** incremental re-parsing is needed for large documents (Phase 2); measure first.
