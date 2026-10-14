# ADR-014: Transclusion (`lmf:include`)

**Status:** proposed (prototype)

**Context.** Spec §4.7: an include must still make sense in the source file, and must handle
cycles, missing targets, access rules and recursion depth.

**Decision.**
* `:::lmf:include {document="…" section="…"|block="…"}` with an optional body that is the
  **fallback text** shown when the include cannot or should not be expanded.
* `document` resolves exactly like a wikilink name, relative to the including note (ADR-011).
  `section` and `block` are mutually exclusive; with neither, the whole body (without front matter)
  is included.
* Expansion is done by the front end that knows the workspace (`Workspace::expand_include`), not by
  the parser. The parser and validator only check the directive's shape.
* **Cycles** (a note that would include itself, directly or indirectly) and nesting deeper than 4
  levels are errors shown in the preview; they are never followed.
* **Missing or ambiguous targets** show a warning card plus the fallback text.
* Included text is *not* copied into the file. The including file stays the source of truth for
  its own content; the included content is shown live from the target's last saved version.
* Plain Markdown export uses the fallback text (or a one-line "Included content: …" note), and
  reports a degraded-conversion loss.

**Access rules.** Not applicable yet (single local folder, no permissions). Revisit with sync and
collaboration (Phase 8).

**Open.** Showing unsaved edits of the target; includes across workspaces; transcluding tasks and
tables as live objects; an expand-on-export option for the CLI (needs a workspace argument).
