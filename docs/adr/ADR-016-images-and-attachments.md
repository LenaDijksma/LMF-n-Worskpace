# ADR-016: Images and attachments

**Status:** proposed

**Context.** Spec §4.15/§9: notes need pictures and files, but loading arbitrary images or opening
arbitrary files is a security and performance risk, and files must stay plain, portable text.

**Decision.**

*Syntax.* Plain Markdown: images `![alt](path)`, attachments as ordinary links to local files
`[text](path)` (a link to a file that is not a note and has an extension). Paths are relative to
the folder of the note that contains them, may be percent-encoded (`my%20file.png`) or written
`<my file.png>`. Nothing LMF-specific is needed, so the source stays portable.

*Where files live.* Anywhere inside the workspace folder. **Attach file…** copies a file into
`<workspace>/assets/` (never overwriting: `name-1.png`, …; unusual characters in the name become
`-`; refused above 64 MiB) and appends the Markdown reference to the document.

*Safety rules (`lmf-workspace::assets`).*
* Only relative paths. Absolute paths, paths that leave the workspace (also through symbolic
  links) and addresses such as `data:`, `file:` or `javascript:` are rejected.
* **Remote images are never fetched.** The preview explains that and shows the address as text.
* Shown images: `png`, `jpg`/`jpeg`, `gif`; at most **4 MiB** and **16 megapixels**. The size is read
  from the file header *before* decoding, so a small file that expands to a huge bitmap is refused
  without being decoded. (SVG and WebP are not shown: SVG can carry scripts and external
  references; WebP support depends on build features.)
* Attachments open only after the user confirms, and only for an allow-list of types (documents,
  spreadsheets, presentations, text/CSV/JSON, images, audio, video, archives). Executables,
  scripts, web pages (`html`), `svg`, `app`, `jar`, `desktop` and everything unknown are not opened.
  The file goes to the system's default program as a single argument, never through a shell.

*Diagnostics.* `LMF018` file not found (warning) · `LMF019` rejected by policy: outside the
workspace, too large, unsupported type or address (warning; info for attachments whose type is
merely not on the safe list) · `LMF020` remote image not loaded (info). Shown in the problems bar
and by `lmf check --assets [--root DIR]`.

*Performance.* Decoded images are cached by file and modification time (at most 32), image cards
are re-rendered from the cache, and nothing is decoded in Edit view.

**Consequences.** Export to plain Markdown needs no conversion (the references are already plain).
Copying a note to another folder keeps working only if its relative paths still resolve; a future
"export with assets" command should bundle them.

**Open.** Pasting images from the clipboard, drag and drop, thumbnails/resizing, WebP and
(sanitised) SVG, image dimensions/captions syntax, orphan-asset cleanup, inline (in-paragraph)
images.
