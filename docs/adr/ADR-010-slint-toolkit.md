# ADR-010: Slint as the native GUI toolkit

**Status:** accepted (project owner's decision)

**Context.** ADR-009 requires a native desktop GUI in Rust. Candidates were egui, Iced and Slint.

**Decision.** Use **Slint** (`slint` 1.x, declarative `.slint` UI files compiled at build time).

**Licensing (checked October 2026 against the Slint repository's license page).** Slint is
offered under any one of: a Royalty-free license (proprietary desktop, mobile and web apps at no
cost; embedded excluded; community discussion says it includes an attribution requirement), GPLv3
(open-source projects, including embedded), or a paid commercial license. Pick one deliberately
when the project's own license is decided (spec open decision 32), and read the license text
before distributing binaries.

**Consequences.**
* UI lives in `crates/lmf-app/ui/*.slint`; Rust only maps `lmf-core` results onto widgets.
* Slint has no built-in rich-text/Markdown widget. The app renders a **card-based preview** from
  `lmf_core::preview` (a UI-independent view model) and will adopt richer text rendering when the
  Markdown engine (ADR-001) lands.
* The std `TextEdit` widget is the first editor. If it proves too limited (syntax highlighting,
  large files, programmatic cursor control), the editor becomes a custom component; this is the
  main technical risk to watch in Phase 2.
