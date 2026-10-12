# Lovely Workspace — LMF core (Phase 1)

This repository implements **Phase 1 — LMF Core** of `LOVELY_PROJECT_MASTER_SPEC.md`:
a Rust library and command-line tool for **LMF (Lovely Markdown Format)**.

The end product is a **native desktop application with a GUI** (not a website), built with
**Slint** (ADR-010) on top of `lmf-core`. `lmf-core` and the CLI stay UI-free.

| Crate | What it is |
|-------|-----------|
| `crates/lmf-core` | Parser, document model with source spans, diagnostics, validator, Markdown import/export. **Zero dependencies.** |
| `crates/lmf-cli`  | The `lmf` command (`check`, `to-markdown`, `from-markdown`, `ast`). |
| `crates/lmf-workspace` | Workspace index: folder scan, link resolution, backlinks, search. Pure logic plus one scanner module. |
| `crates/lmf-app`  | **Phase 2:** the `lovely` desktop app (Slint): folder navigation, search, editor, preview, diagnostics, links/backlinks, autosave. |

> **Status.** Phase 1 (`lmf-core`, `lmf-cli`) and the first Phase 2 slice of `lmf-app` build and
> run. The rest of Phase 2 (`lmf-workspace`, links, sidebar, search, autosave, themes, shortcuts,
> clickable preview links) was written without a compiler and has not been built yet.

## Build and test

```sh
cargo test                         # core, workspace, CLI and app tests
cargo run -p lmf-cli -- --help     # command-line tool
cargo run -p lmf-app               # desktop app (opens a sample document)
cargo run -p lmf-app -- notes.lmf  # desktop app, opening a file
cargo run -p lmf-app -- ~/notes    # desktop app, opening a folder
```

## Using the CLI

```sh
lmf check notes.lmf                      # diagnostics as file:line:col: severity[CODE]: message
lmf check --strict --format json *.lmf   # CI-friendly; exit 1 on errors (or warnings with --strict)
lmf to-markdown notes.lmf -o notes.md    # plain Markdown; every loss is listed on stderr
lmf to-markdown --unknown drop --front-matter strip notes.lmf   # destructive options are explicit
lmf from-markdown --stamp README.md      # GitHub alerts -> callouts, adds lmf_version front matter
lmf ast notes.lmf                        # show the parsed structure
```

## What Phase 1 covers

* **Source-preserving parse.** Any text is valid LMF. `Document::to_source()` always returns the
  input byte for byte; every node has a byte `Span`. Tested by round-trip and random-input tests.
* **Front matter** (small YAML subset) with duplicate-key and malformed-line diagnostics.
* **Directives** `:::namespace:name {attrs}` … `:::` with nesting, typed attributes, code-fence
  awareness, and recovery from malformed or unclosed blocks (content is never dropped).
* **Unknown extensions** are preserved verbatim and reported as `LMF006` info (typos inside the
  reserved `lmf:` namespace are warnings).
* **Built-in extensions:** `lmf:callout`, `lmf:details`, `lmf:tabs` / `lmf:tab`.
* **Validator** with stable diagnostic codes `LMF001`–`LMF017` (see `docs/lmf-0.1-implemented-spec.md`).
* **Converters** with loss reporting: LMF → plain Markdown, Markdown (GitHub alerts) → LMF.
* **Fixture corpus** in `crates/lmf-core/tests/fixtures/` (`.lmf` + `.expect` per case).

## Desktop app (Phase 2)

* **Open folder…** scans for `.md`, `.markdown` and `.lmf` files (hidden files and symlinks are
  skipped; unreadable/non-UTF-8/oversized files are counted, not hidden). Click a note to open it.
* **Search** box: all terms must match title, aliases, tags, headings, path or text; best matches first.
* **Editor + structural preview + live diagnostics.** Inline Markdown (bold, links) is shown as typed
  until the Markdown engine lands.
* **Links and backlinks panel** for the open note: `[[wikilinks]]` and relative Markdown links,
  resolved case-insensitively by file name, title or alias (ADR-011). Ambiguous and missing targets
  are flagged, never guessed. Click a link or backlink to jump.
* **Autosave** every 2 seconds to the file itself, with visible status. Files stay plain text on
  disk, readable and editable by any other tool.
* **Conflict detection:** if a file changed on disk since you opened it, autosave stops and says
  so; a manual Save asks before overwriting.
* **Safe closing:** closing or switching files saves first, or asks before discarding an unsaved,
  never-saved document.

* **Themes and text size:** Light, Dark and High contrast (contrast ratios are enforced by tests),
  text scaling 80–200%. Choices are remembered (ADR-012).
* **Keyboard shortcuts** (Ctrl, or Cmd on macOS): `S` save · `O` open file · `Shift+O` open folder ·
  `F` search · `+` / `-` text size · `Shift+T` cycle theme.
* **Clickable links in the preview.** Internal links jump to the note; external `http(s)` links ask
  for confirmation first. Raw HTML is shown as inert text; nothing is fetched automatically.
* **Accessibility baseline:** screen-reader roles and labels on lists and links, colour never the
  only signal. Not yet tested with real assistive technology; no full keyboard navigation of lists.

Not yet: tabs/multiple open documents, renaming with link updates, image/attachment handling,
keyboard navigation of the file list.

On Linux, Slint may need system packages (e.g. `libfontconfig1-dev`, `pkg-config`) if the build
reports missing libraries.

## What it deliberately does not do yet

* It does **not** parse inline Markdown (emphasis, links, tables…). Markdown regions are opaque
  blocks until the Markdown baseline engine is chosen (ADR-001). Nothing is lost meanwhile.
* Search, boards, API blocks, automation or sync (Phases 3–8); the app is only the first Phase 2 slice.
* Directives inside list items or blockquotes are not recognized (top level and directive
  bodies only).
* Link targets are not rewritten when a note is renamed or moved.

## Decisions made on the spec's open questions

Recorded in `docs/adr/`. All are reversible and marked *proposed*.

* ADR-001 Markdown baseline · ADR-002 Directive grammar · ADR-003 Metadata syntax
* ADR-005 Source-preserving parser · ADR-009 Native desktop GUI · ADR-010 Slint toolkit · ADR-011 Links and search · ADR-012 Safe content and accessibility

Licensing is undecided (spec open decision 32), so the crates are `publish = false` and carry no
license field.
