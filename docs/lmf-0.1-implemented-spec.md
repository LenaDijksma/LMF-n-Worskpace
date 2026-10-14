# LMF 0.1 — what `lmf-core` implements

Status: **draft**, implemented by `lmf-core` 0.1.0. Where this differs from the master spec's
sketches, this document is the contract for the code.

## 1. Source

UTF-8 text. A leading BOM is ignored. `\n` and `\r\n` line endings are accepted. Invalid UTF-8
is diagnosed (`LMF001`); implementations must not write such a document back.

## 2. Front matter

A block at the very start: line `---`, metadata lines, then `---` (or `...`).

Treated as front matter only if the first non-blank, non-comment line inside looks like
`key: …`; otherwise the `---` lines are ordinary Markdown thematic breaks.

Supported syntax: `key: scalar`, `key: [a, "b"]`, block lists (`key:` + `- item` lines), `#`
comment lines. Scalars: quoted strings, integers, `true`/`false`, `null`, bare strings.
Unsupported (diagnosed `LMF003`): nested mappings, multi-line scalars, trailing comments.
Unknown keys are preserved. Duplicate keys: `LMF004`.

Known fields (validated): `lmf_version` (positive integer, must be ≤ 1), `title`, `id`, `author`,
`description`, `license` (strings), `language` (BCP 47-shaped), `created`/`modified` (ISO 8601
date or timestamp), `tags`, `aliases`, `extensions` (lists of strings).

## 3. Directives

```text
opener = 0*3SP 3*COLON name [ 1*WS attr-block ] *WS EOL
closer = 0*3SP 3*COLON *WS EOL
name   = ns ":" ident              ; both lowercase: [a-z][a-z0-9_-]*
```

* A closer closes the innermost open directive **if it has at least as many colons as that
  directive's opener**. Otherwise it is plain content. To show a literal `:::` line, use a
  longer outer fence.
* Same-length nesting is allowed (`lmf:tabs` containing `lmf:tab`, each closed by `:::`).
* Delimiters inside fenced code blocks (``` / ~~~) are plain text. Indented 4+ spaces = code.
* `:::word` where `word` is not `ns:name` is Markdown, with warning `LMF007` (so Docusaurus-style
  admonitions are not silently misread). A bare `:::` with nothing to close: warning `LMF007`.
* Unclosed directive: closed at end of input, error `LMF007`; content kept.
* Attributes must be on the opening line.

### Attribute block

`{key="string" flag=true n=3 x=1.5}` — keys `[A-Za-z_][A-Za-z0-9_.-]*`; values are quoted strings
(escapes `\"` `\\` `\n` `\t`), `true`/`false`, integers, or floats. Strings must be quoted; bare
words are errors (`LMF008`). Duplicate keys: first wins, `LMF010`.

Every directive may carry `id` (letters, digits, `_ - . :`; unique per document, `LMF005`) and
`version` (extension payload version).

## 4. Built-in extensions

| Name | Attributes | Rules |
|------|-----------|-------|
| `lmf:callout` | `kind` ∈ note, tip, info, success, warning, danger, question, example; `title` | body required |
| `lmf:details` | `title`, `open` (bool) | body required |
| `lmf:tabs` | – | may contain only `lmf:tab` |
| `lmf:tab` | `title` (**required**) | must be directly inside `lmf:tabs` |
| `lmf:include` | `document` (**required**), `section`, `block` (not both) | body (optional) is the fallback text; see §4.1 |
| `lmf:math` | `format` ∈ latex | body required; source is shown, rendering is optional |
| `lmf:diagram` | `type` (**required**) ∈ mermaid | body required; source is never executed; rendering is optional |

### 4.1 Block IDs and transclusion

A **block ID** is a marker `^id` at the end of a line (it labels that paragraph) or alone on a line
(it labels the block above). IDs are `^` plus 2+ characters of `[A-Za-z0-9_-]` starting with an
alphanumeric, after whitespace or at line start (`x^2` is not an ID). Markers stay in the text.
IDs share one namespace with directive `id` attributes; duplicates are `LMF005`.
Links use `[[Doc#block:id]]`; an include uses `block="id"`.

`lmf:include {document="…" section="…"}` pulls the named heading's section (the heading and
everything up to the next heading of the same or a higher level), or one block, or the whole
document body into the preview. `document` resolves like a wikilink (ADR-011). Includes may nest
up to 4 levels; cycles and missing targets are shown as warnings with the author's fallback text
and are never followed. Plain-Markdown export uses the fallback text (ADR-014).

Everything else is an **unknown extension**: preserved verbatim, never executed, `LMF006`.

## 5. Diagnostics

| Code | Meaning | Typical severity |
|------|---------|------------------|
| LMF001 | invalid UTF-8 | error |
| LMF002 | `lmf_version` newer than supported | error |
| LMF003 | malformed metadata | error |
| LMF004 | duplicate metadata key | error |
| LMF005 | duplicate directive id | error |
| LMF006 | unknown extension (typo in `lmf:` namespace = warning) | info / warning |
| LMF007 | malformed directive (unclosed, stray closer, bad name) | error / warning |
| LMF008 | invalid attribute syntax | error |
| LMF009 | invalid attribute value or type | error |
| LMF010 | duplicate attribute | error |
| LMF011 | unknown attribute on a known extension | warning |
| LMF012 | invalid nesting | error / warning |
| LMF013 | empty body | warning |
| LMF014 | missing required attribute | error |
| LMF015 | unclosed code fence | warning |
| LMF016 | extension payload version newer than supported | warning |
| LMF017 | invalid metadata value | error / warning |

Codes are stable; never renumber.

## 6. Conversion

* **To plain Markdown:** callout → blockquote with bold label (matches spec §4.19); details and
  tabs → bold titles followed by content; unknown extensions kept as literal text (default) or
  dropped (`--unknown drop`). Front matter kept (default) or stripped. Every change is a `Loss`
  (`note` / `degraded` / `dropped`).
* **From Markdown:** GitHub alerts (`[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`,
  `[!CAUTION]`) → `lmf:callout`. Optional `lmf_version` stamp.

## 7. Conformance claimed

Core Reader (preserves unknown extensions), Core Writer for export/import paths that never
rewrite untouched regions, and Validator for the rules above. No Runtime. No claim to full
CommonMark/GFM parsing yet (ADR-001).
