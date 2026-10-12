# ADR-011: Internal links, backlinks and search

**Status:** proposed

**Context.** Spec §4.6 lists questions a link design must answer (duplicate titles, paths,
escaping, renames, aliases, case, missing targets). Phase 2 needs basic internal links and
backlinks.

**Decision.**

*Syntax (extracted by `lmf_core::links`)*: `[[Target]]`, `[[Target|label]]`, `[[Target#Heading]]`,
`[[Target#block:id]]`, and ordinary relative Markdown links to `.md` / `.lmf` / `.markdown` files.
Links inside fenced code, inline code and unknown extension blocks are ignored. A backslash
escapes `[`. `[[#Heading]]` links within the same note.

*Resolution (implemented in `lmf-workspace`)*, all case-insensitive:
1. Markdown links and wikilinks containing `/` or a note extension are paths: tried relative to
   the linking note's folder, then relative to the workspace root; `.md`, `.lmf`, `.markdown`
   are tried if no extension is given. Paths can never escape the workspace root.
2. Otherwise a name is matched in tiers: file name (without extension), then title (front matter
   `title`, else first `#` heading, else file name), then `aliases`.
3. If a tier matches several notes, a single match in the linking note's own folder wins;
   otherwise the link is **ambiguous** and is shown as such. It is never guessed.
4. No match = **unresolved**; shown, never an error. A missing heading is flagged separately.

*Backlinks* are derived from resolved links (self-links excluded) with the linking line as
context. Ambiguous and unresolved links create no backlinks.

*Search* is a case-insensitive AND of whitespace-separated terms over title, aliases, tags,
headings, path and body. Ranking: exact title > title > alias/tag/heading > path > body hits.

*Renames.* The workspace index is derived data rebuilt from files. Renaming a note does **not**
rewrite links in other notes yet; links by title survive a file rename, links by path do not.

**Open.** Block-reference indexing (`#block:`), link rewriting on rename, quoting `|` and `]]`
inside targets, and a stored (incremental) index for very large workspaces.
