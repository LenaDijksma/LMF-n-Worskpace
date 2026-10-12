# ADR-012: Safe content handling, appearance and accessibility basics

**Status:** proposed

## Safe HTML and external content

* **Raw HTML is never interpreted.** The preview shows HTML as the literal text it is. No
  `<script>`, no inline event handlers, no styles, no embedded frames, no remote images. This is
  enforced by construction (the preview is built from text cards, not a web engine) and by the
  test `preview::tests::raw_html_is_inert_text`.
* **Nothing is fetched automatically.** Parsing, validating and previewing never touch the network.
* **External links are explicit.** `[text](https://…)` and `<https://…>` appear as clickable chips
  marked with an arrow. Clicking one asks for confirmation and shows the full URL. Only plain
  `http://` / `https://` URLs without credentials, whitespace or control characters pass
  (`external::is_safe_url`); `javascript:`, `file:`, `mailto:` and others are never opened. The URL
  is handed to the OS opener as a single argument, never through a shell.
* **Unknown extensions are inert**: shown as source, never executed.

*Later (Phase 3+):* if rich HTML or embedded content is ever supported, it needs a sanitizing
allow-list and a per-document trust decision, not a relaxation of the rules above.

## Appearance

* Three themes: Light, Dark, High contrast. Palettes are data in `src/theme.rs` and **contrast is
  tested**: at least 4.5:1 for text and meaningful accents (7:1 in High contrast).
* Text scaling from 80% to 200% (also scales sidebar, preview and diagnostics).
* Settings (theme, text scale, last folder) live in a small plain-text file in the platform config
  folder (`…/lovely-workspace/settings.txt`). Nothing else is stored; nothing leaves the machine.

## Keyboard and assistive technology

* Shortcuts (Ctrl, or Cmd on macOS): `S` save · `O` open file · `Shift+O` open folder · `F` search ·
  `+` / `-` text size · `Shift+T` cycle theme.
* List rows and link chips expose button roles and text labels to screen readers.
* Colour is never the only signal: diagnostics carry the words error/warning/info, links carry
  arrows/marks (`→` resolved, `✗` not found, `⚠` ambiguous, `↗` external).

## Known gaps

No full keyboard navigation of the file list and preview chips yet (mouse/touch and screen-reader
actions work; Tab-order and arrow-key navigation do not). No right-to-left layout review, no
reduced-motion setting (there is no animation). Screen-reader behaviour has not been tested on real
assistive technology. Treat the accessibility work as a baseline, not a compliance claim.
