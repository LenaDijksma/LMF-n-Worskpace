# ADR-015: Rich text in the preview

**Status:** proposed

**Context.** The preview originally showed Markdown source as typed (`**bold**`, `|` tables, raw
`*` bullets). Slint 1.16 added a `StyledText` element that renders a Markdown subset.

**What `StyledText` renders (per Slint's documentation):** bold, italic, strikethrough, underline,
inline code, links, ordered and unordered lists, colours. **Not supported:** headings, tables,
block quotes, horizontal rules, images, most HTML. It reports link clicks via `link-clicked`.

**Decision.** Split the work between the core library and the widget:

* `lmf_core::preview` turns a document into *block cards*: headings (ATX and setext), paragraphs,
  fenced code, **tables** (GitHub style: alignment, escaped pipes, ragged rows normalised),
  **block quotes**, **rules**, callouts, tabs, includes, math, diagrams.
* `lmf_core::richtext::display_markdown` prepares *inline* Markdown for the widget and enforces
  the safety policy (ADR-012): HTML is escaped, only `http(s)` links stay links, `[[wikilinks]]`
  and note links become internal links (or struck-through text when the target is missing), images
  become an `(image: alt)` placeholder, task items get ☐ / ☑.
* The app renders each card with `StyledText` (or plain monospace text for code), draws tables as
  a grid of `StyledText` cells with theme-coloured borders, and routes link clicks: `lmf:N` jumps
  to the note behind it; `http(s)` links ask for confirmation first.
* If `StyledText` rejects a piece of Markdown, that piece is shown as plain text instead of
  failing.

**Consequences.**
* Links inside the preview are clickable (the separate "link chips" were removed).
* Table columns have equal width; there is no horizontal scrolling for very wide tables yet.
* Nested block quotes show inner `>` markers literally. Footnotes, definition lists and
  reference-style links are not rendered.
* Rendering depends on Slint's `StyledText`, which Slint describes as a first step toward rich
  text; behaviour may change between Slint releases.

**Performance.** Rebuilding rich text for every card on every keystroke made typing laggy on long
documents. The preview is now rebuilt 250 ms after typing pauses, and only the cards that actually
changed are re-rendered (`update_preview` diffs the old and new card lists; unchanged rows keep
their widgets). Internal link destinations are self-describing (`lmf:<encoded path>`), so unchanged
cards never carry stale state; a theme, text-size, workspace or open-note change forces a full
rebuild.

**Separate Edit and Preview views.** Rendering while typing was still the main cost, so the editor and
the preview are now separate views (one visible at a time, both kept alive). Edit view does no
rendering; Preview view is rendered when you switch to it, using the same changed-cards-only update.
Only the cheap problems list refreshes while typing.

**Open.** Real images (Phase 3, next slice), horizontal scrolling and column sizing for tables,
syntax highlighting in code blocks, selecting/copying rendered text.
