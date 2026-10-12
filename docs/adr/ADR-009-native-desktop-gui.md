# ADR-009: The application is a native desktop GUI, not a website

**Status:** accepted (project owner's decision)

**Context.** The spec leaves platform and UI toolkit open (open decisions 13–16).

**Decision.** The workspace application will be a **native desktop application with a GUI**,
written in Rust on top of `lmf-core`. It is not a website or web app, and should not depend on a
browser engine or web view.

**Toolkit:** decided in ADR-010 (Slint).

**Still open.** Which editor widget long-term, and which platforms first.

**Consequences.** `lmf-core` stays UI-free so the CLI, tests and the GUI share one implementation.
