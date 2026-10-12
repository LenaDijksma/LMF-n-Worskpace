//! Lovely Workspace — native desktop application (Phase 2).
//!
//! Open a folder of notes, navigate and search them, edit with a live structural preview and
//! diagnostics, follow internal links and backlinks, and rely on autosave with visible status.
//! Themes (including high contrast), text scaling, keyboard shortcuts and accessible labels are
//! built in. All document and workspace logic lives in `lmf-core` and `lmf-workspace`; this crate
//! only maps their results onto Slint widgets.

mod external;
mod settings;
mod theme;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use lmf_core::links::{extract_all_links, Fragment, Link, LinkKind};
use lmf_core::preview::{build_cards, Card, CardKind};
use lmf_core::{check, parse, LineIndex, Severity};
use lmf_workspace::{scan, NoteSource, Resolution, Workspace};
use slint::{Color, ComponentHandle, ModelRc, VecModel};

use settings::{step_scale, Settings};
use theme::{Colors, Rgb};

slint::include_modules!();

const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(2);

const SAMPLE: &str = "\
---
lmf_version: 1
title: Welcome to Lovely Workspace
tags: [notes, demo]
---

# Welcome

Use **Open folder…** (Ctrl+Shift+O) to browse a folder of notes. Link notes with [[wikilinks]]
and see backlinks in the sidebar. External links such as [the Slint site](https://slint.dev)
only open after you confirm.

:::lmf:callout {kind=\"tip\" title=\"Try editing\"}
Change this text, or break the syntax, and watch the diagnostics below. Edits to files are
autosaved. Ctrl+S saves, Ctrl+F searches, Ctrl +/- changes the text size, Ctrl+Shift+T changes
the theme.
:::

:::lmf:tabs
:::lmf:tab {title=\"Linux\"}
Linux instructions.
:::
:::lmf:tab {title=\"Windows\"}
Windows instructions.
:::
:::

:::vendor:interactive-map {version=\"3\"}
Unknown extensions are preserved, never executed.
:::
";

/// Everything the application remembers between UI events.
struct App {
    settings: Settings,
    root: Option<PathBuf>,
    workspace: Option<Workspace>,
    /// Absolute path of the open file; `None` for the unsaved sample/new document.
    path: Option<PathBuf>,
    dirty: bool,
    /// Modification time and size of the file as we last read or wrote it (conflict detection).
    disk_stamp: Option<(SystemTime, u64)>,
    query: String,
}

#[derive(Debug, PartialEq, Eq)]
enum WriteOutcome {
    Written,
    /// Nothing written because the document has no file yet (autosave only).
    NoPath,
    /// The file changed on disk since we loaded it, and the user did not agree to overwrite.
    Conflict,
    /// The user dismissed the file dialog.
    Cancelled,
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let settings = Settings::load();
    apply_theme(&window, &settings);

    let state = Rc::new(RefCell::new(App {
        settings,
        root: None,
        workspace: None,
        path: None,
        dirty: false,
        disk_stamp: None,
        query: String::new(),
    }));

    // Start with a file or folder from the command line, else the last folder, else the sample.
    {
        let mut st = state.borrow_mut();
        let arg = std::env::args().nth(1).map(PathBuf::from);
        let last = st
            .settings
            .last_folder
            .clone()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        match (arg, last) {
            (Some(p), _) if p.is_dir() => {
                open_folder(&window, &mut st, p);
                show_empty(&window, &st);
            }
            (Some(p), _) => {
                if let Err(e) = load_file(&window, &mut st, p) {
                    window.set_status(e.as_str().into());
                }
            }
            (None, Some(dir)) => {
                open_folder(&window, &mut st, dir);
                show_empty(&window, &st);
            }
            (None, None) => {
                window.set_editor_text(SAMPLE.into());
                refresh(&window, SAMPLE, &st);
            }
        }
    }

    // --- editing --------------------------------------------------------------------------
    window.on_text_edited({
        let weak = window.as_weak();
        let state = state.clone();
        move |text| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            st.dirty = true;
            refresh(&w, text.as_str(), &st);
            let message = if st.path.is_some() {
                "Unsaved changes (autosave on)"
            } else {
                "Unsaved new document"
            };
            w.set_save_status(message.into());
        }
    });

    // --- files ----------------------------------------------------------------------------
    window.on_open_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            if !leave_current(&w, &mut st) {
                return;
            }
            let picked = rfd::FileDialog::new()
                .add_filter("LMF / Markdown", &["lmf", "md", "markdown", "txt"])
                .pick_file();
            if let Some(path) = picked {
                if let Err(e) = load_file(&w, &mut st, path) {
                    w.set_status(e.as_str().into());
                }
            }
        }
    });

    window.on_save_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            report_write(&w, &mut st, false);
        }
    });

    window.on_open_folder_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                open_folder(&w, &mut st, dir);
            }
        }
    });

    // --- navigation, search, links --------------------------------------------------------
    window.on_search_edited({
        let weak = window.as_weak();
        let state = state.clone();
        move |text| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            st.query = text.to_string();
            update_side(&w, &st);
        }
    });

    window.on_nav_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move |rel| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            go_to(&w, &mut st, rel.as_str());
        }
    });

    window.on_link_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move |rel| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            go_to(&w, &mut st, rel.as_str());
        }
    });

    window.on_preview_link_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move |target, is_external| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            if is_external {
                open_external(&w, target.as_str());
            } else {
                go_to(&w, &mut st, target.as_str());
            }
        }
    });

    // --- appearance -----------------------------------------------------------------------
    window.on_theme_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            let next = st.settings.theme.next();
            st.settings.theme = next;
            let _ = st.settings.save();
            restyle(&w, &st);
        }
    });

    window.on_font_larger({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            let scale = step_scale(st.settings.font_scale, 1);
            st.settings.font_scale = scale;
            let _ = st.settings.save();
            restyle(&w, &st);
        }
    });

    window.on_font_smaller({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            let scale = step_scale(st.settings.font_scale, -1);
            st.settings.font_scale = scale;
            let _ = st.settings.save();
            restyle(&w, &st);
        }
    });

    // --- autosave -------------------------------------------------------------------------
    // The timer must stay alive until the event loop ends.
    let autosave = slint::Timer::default();
    autosave.start(slint::TimerMode::Repeated, AUTOSAVE_INTERVAL, {
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            if st.dirty && st.path.is_some() {
                report_write(&w, &mut st, true);
            }
        }
    });

    // --- closing --------------------------------------------------------------------------
    window.window().on_close_requested({
        let state = state.clone();
        let weak = window.as_weak();
        move || {
            let Some(w) = weak.upgrade() else {
                return slint::CloseRequestResponse::HideWindow;
            };
            let Ok(mut st) = state.try_borrow_mut() else {
                return slint::CloseRequestResponse::KeepWindowShown;
            };
            if leave_current(&w, &mut st) {
                slint::CloseRequestResponse::HideWindow
            } else {
                slint::CloseRequestResponse::KeepWindowShown
            }
        }
    });

    let result = window.run();
    drop(autosave);
    result
}

// ---------------------------------------------------------------------------------------------
// Application logic
// ---------------------------------------------------------------------------------------------

fn confirm(title: &str, description: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// Opens an external link only if it is a plain http(s) URL and the user confirms.
fn open_external(w: &MainWindow, url: &str) {
    if !external::is_safe_url(url) {
        w.set_save_status("Blocked an unsafe link".into());
        return;
    }
    let question = format!("This will open the link in your web browser:\n\n{}", url);
    if !confirm("Open external link?", &question) {
        return;
    }
    if let Err(e) = external::open(url) {
        w.set_save_status(e.as_str().into());
    }
}

/// Makes sure the current document can be replaced: saves it if it has a file, otherwise asks
/// before discarding. Returns `false` if the user should stay where they are.
fn leave_current(w: &MainWindow, app: &mut App) -> bool {
    if !app.dirty {
        return true;
    }
    if app.path.is_some() {
        if let Ok(WriteOutcome::Written) = write_current(w, app, true) {
            return true;
        }
        return confirm(
            "Could not save",
            "The current file could not be saved automatically. Discard your unsaved changes?",
        );
    }
    confirm(
        "Unsaved changes",
        "This document has not been saved to a file. Discard it?",
    )
}

/// Opens a note by its workspace-relative path (from the file list, search or a link).
fn go_to(w: &MainWindow, app: &mut App, rel: &str) {
    if rel.is_empty() {
        return;
    }
    let Some(root) = app.root.clone() else { return };
    if !leave_current(w, app) {
        return;
    }
    if let Err(e) = load_file(w, app, root.join(rel)) {
        w.set_status(e.as_str().into());
    }
}

fn open_folder(w: &MainWindow, app: &mut App, dir: PathBuf) {
    match scan(&dir) {
        Ok(result) => {
            let count = result.notes.len();
            let skipped = result.skipped.len();
            app.workspace = Some(Workspace::build(result.notes));
            app.root = Some(dir.clone());
            app.query.clear();
            if let Some(s) = dir.to_str() {
                app.settings.last_folder = Some(s.to_string());
                let _ = app.settings.save();
            }
            let mut label = format!("{} \u{2014} {} notes", dir.display(), count);
            if skipped > 0 {
                label.push_str(&format!(" ({} skipped)", skipped));
            }
            w.set_folder_label(label.as_str().into());
            update_side(w, app);
        }
        Err(e) => {
            let msg = format!("Cannot open folder {}: {}", dir.display(), e);
            w.set_status(msg.as_str().into());
        }
    }
}

/// Shows an empty editor (used after opening a folder, before a note is chosen).
fn show_empty(w: &MainWindow, app: &App) {
    w.set_editor_text("".into());
    refresh(w, "", app);
}

fn load_file(w: &MainWindow, app: &mut App, path: PathBuf) -> Result<(), String> {
    let bytes =
        std::fs::read(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{} is not valid UTF-8; not opened", path.display()))?;
    w.set_editor_text(text.as_str().into());
    app.disk_stamp = stamp(&path);
    app.path = Some(path);
    app.dirty = false;
    w.set_save_status("".into());
    refresh(w, &text, app);
    update_side(w, app);
    Ok(())
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Writes the editor text to the current file (asking for a path if there is none and this is
/// not an autosave). Refuses to overwrite a file that changed on disk unless the user agrees.
fn write_current(w: &MainWindow, app: &mut App, autosave: bool) -> Result<WriteOutcome, String> {
    let path = match &app.path {
        Some(p) => p.clone(),
        None => {
            if autosave {
                return Ok(WriteOutcome::NoPath);
            }
            let start = app.root.clone();
            let mut dialog = rfd::FileDialog::new()
                .add_filter("LMF / Markdown", &["lmf", "md", "markdown"])
                .set_file_name("untitled.lmf");
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            match dialog.save_file() {
                Some(p) => {
                    app.disk_stamp = None;
                    p
                }
                None => return Ok(WriteOutcome::Cancelled),
            }
        }
    };

    // Conflict detection: has the file changed since we last read or wrote it?
    if let (Some(known), Some(now)) = (app.disk_stamp, stamp(&path)) {
        if known != now {
            if autosave {
                return Ok(WriteOutcome::Conflict);
            }
            let agreed = confirm(
                "File changed on disk",
                "This file was modified outside the app since you opened it. Overwrite it with your version?",
            );
            if !agreed {
                return Ok(WriteOutcome::Conflict);
            }
        }
    }

    let text = w.get_editor_text();
    std::fs::write(&path, text.as_str())
        .map_err(|e| format!("Cannot save {}: {}", path.display(), e))?;
    app.disk_stamp = stamp(&path);
    app.path = Some(path.clone());
    app.dirty = false;

    // Keep the workspace index in step with what is on disk.
    if let (Some(root), Some(ws)) = (&app.root, &mut app.workspace) {
        if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel_to_string(rel);
            if is_note_path(&rel) {
                ws.upsert_note(NoteSource {
                    path: rel,
                    text: text.to_string(),
                });
            }
        }
    }
    Ok(WriteOutcome::Written)
}

fn rel_to_string(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join("/")
}

fn is_note_path(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".lmf")
}

/// Runs a write and reports the outcome in the UI.
fn report_write(w: &MainWindow, app: &mut App, autosave: bool) {
    let message = match write_current(w, app, autosave) {
        Ok(WriteOutcome::Written) => {
            let text = w.get_editor_text();
            refresh(w, text.as_str(), app);
            update_side(w, app);
            if autosave {
                "Autosaved".to_string()
            } else {
                format!("Saved {}", file_name(app))
            }
        }
        Ok(WriteOutcome::Conflict) => {
            "File changed on disk \u{2014} not saved. Save to choose, or reopen the file."
                .to_string()
        }
        Ok(WriteOutcome::NoPath) | Ok(WriteOutcome::Cancelled) => return,
        Err(e) => e,
    };
    w.set_save_status(message.as_str().into());
}

fn file_name(app: &App) -> String {
    app.path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".to_string())
}

/// Workspace-relative path of the open file, if it lives inside the open folder.
fn current_rel(app: &App) -> Option<String> {
    let root = app.root.as_ref()?;
    let path = app.path.as_ref()?;
    let rel = path.strip_prefix(root).ok()?;
    Some(rel_to_string(rel))
}

/// Index of the open note in the workspace, if it is part of it.
fn current_note_index(app: &App) -> Option<usize> {
    let ws = app.workspace.as_ref()?;
    let rel = current_rel(app)?;
    ws.note_index(&rel)
}

// ---------------------------------------------------------------------------------------------
// View updates
// ---------------------------------------------------------------------------------------------

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn col(c: Rgb) -> Color {
    Color::from_rgb_u8(c.0, c.1, c.2)
}

fn transparent() -> Color {
    Color::from_argb_u8(0, 0, 0, 0)
}

/// Pushes the current theme and text scale into the UI.
fn apply_theme(w: &MainWindow, settings: &Settings) {
    let c = settings.theme.colors();
    let theme = w.global::<Theme>();
    theme.set_window_bg(col(c.window_bg));
    theme.set_sidebar_bg(col(c.sidebar_bg));
    theme.set_sidebar_text(col(c.sidebar_text));
    theme.set_sidebar_muted(col(c.sidebar_muted));
    theme.set_hover(col(c.hover));
    theme.set_panel_bg(col(c.panel_bg));
    theme.set_text(col(c.text));
    theme.set_link(col(c.link));
    theme.set_diag_bg(col(c.diag_bg));
    theme.set_scale(settings.font_scale);
    w.set_theme_name(settings.theme.name().into());
    w.set_dark_scheme(settings.theme.is_dark());
    w.set_editor_font_size(14.0 * settings.font_scale);
}

/// Re-applies appearance and rebuilds everything that carries colours or sizes.
fn restyle(w: &MainWindow, app: &App) {
    apply_theme(w, &app.settings);
    let text = w.get_editor_text();
    refresh(w, text.as_str(), app);
    update_side(w, app);
}

/// Re-parses `text` and updates preview, diagnostics, status line and window title.
fn refresh(w: &MainWindow, text: &str, app: &App) {
    let colors = app.settings.theme.colors();
    let scale = app.settings.font_scale;
    let doc = parse(text);
    let diags = check(&doc);
    let index = LineIndex::new(&doc.source);
    let from = current_note_index(app);

    let items: Vec<PreviewItem> = build_cards(&doc)
        .iter()
        .map(|card| {
            let links = card_links(card, app, colors, from);
            to_item(card, colors, scale, links)
        })
        .collect();
    w.set_preview_items(model(items));

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut rows: Vec<DiagItem> = Vec::new();
    for d in &diags {
        let tint = match d.severity {
            Severity::Error => {
                errors += 1;
                col(colors.error)
            }
            Severity::Warning => {
                warnings += 1;
                col(colors.warning)
            }
            Severity::Info => col(colors.info),
        };
        let (line, col_no) = index.line_col(&doc.source, d.span.start);
        let row = format!(
            "{:<7} {}:{}   [{}] {}",
            d.severity.as_str(),
            line,
            col_no,
            d.code,
            d.message
        );
        rows.push(DiagItem {
            text: row.as_str().into(),
            tint,
        });
    }
    if rows.is_empty() {
        rows.push(DiagItem {
            text: "No problems found.".into(),
            tint: col(colors.ok),
        });
    }
    w.set_diagnostics(model(rows));

    let status = if errors == 0 && warnings == 0 {
        "No problems".to_string()
    } else {
        format!("{} error(s), {} warning(s)", errors, warnings)
    };
    w.set_status(status.as_str().into());

    let title = format!(
        "{}{} \u{2014} Lovely Workspace",
        file_name(app),
        if app.dirty { " \u{2022}" } else { "" }
    );
    w.set_window_title(title.as_str().into());
}

/// Clickable links found in a card's text. Internal links are resolved against the workspace;
/// external links carry their URL and are only opened after confirmation.
fn card_links(card: &Card, app: &App, colors: &Colors, from: Option<usize>) -> Vec<PreviewLink> {
    match card.kind {
        CardKind::Paragraph | CardKind::Callout(_) | CardKind::Details | CardKind::Tab => {}
        _ => return Vec::new(),
    }
    if !card.body.contains('[') && !card.body.contains('<') {
        return Vec::new();
    }
    let doc = parse(&card.body);
    let mut out = Vec::new();
    for link in extract_all_links(&doc) {
        if link.kind == LinkKind::External {
            let label = link.label.clone().unwrap_or_else(|| link.target.clone());
            let text = format!("\u{2197} {}", label);
            out.push(PreviewLink {
                text: text.as_str().into(),
                target: link.target.as_str().into(),
                external: true,
                tint: col(colors.link),
            });
            continue;
        }
        let resolved = match (&app.workspace, from) {
            (Some(ws), Some(f)) => match ws.resolve(f, &link) {
                Resolution::Resolved(t) => Some(ws.notes()[t].path.clone()),
                _ => None,
            },
            _ => None,
        };
        let shown = link_display(&link);
        match resolved {
            Some(path) => {
                let text = format!("\u{2192} {}", shown);
                out.push(PreviewLink {
                    text: text.as_str().into(),
                    target: path.as_str().into(),
                    external: false,
                    tint: col(colors.link),
                });
            }
            None => {
                let text = format!("\u{2717} {}", shown);
                out.push(PreviewLink {
                    text: text.as_str().into(),
                    target: "".into(),
                    external: false,
                    tint: col(colors.error),
                });
            }
        }
    }
    out
}

/// Updates the sidebar: file list or search results, and links/backlinks of the open note.
fn update_side(w: &MainWindow, app: &App) {
    let colors = app.settings.theme.colors();
    let mut nav: Vec<NavItem> = Vec::new();
    if let Some(ws) = &app.workspace {
        if app.query.trim().is_empty() {
            for n in ws.notes() {
                nav.push(NavItem {
                    label: n.title.as_str().into(),
                    detail: n.path.as_str().into(),
                    path: n.path.as_str().into(),
                });
            }
        } else {
            for hit in ws.search(&app.query, 100) {
                let n = &ws.notes()[hit.note];
                let detail = if hit.snippet.is_empty() {
                    n.path.clone()
                } else {
                    format!("{} \u{00b7} {}", n.path, hit.snippet)
                };
                nav.push(NavItem {
                    label: n.title.as_str().into(),
                    detail: detail.as_str().into(),
                    path: n.path.as_str().into(),
                });
            }
        }
    }
    w.set_nav_items(model(nav));

    let mut links: Vec<LinkItem> = Vec::new();
    if let (Some(ws), Some(idx)) = (&app.workspace, current_note_index(app)) {
        let outgoing = ws.outgoing(idx);
        links.push(header(format!("Outgoing links ({})", outgoing.len()), colors));
        for (link, resolution) in &outgoing {
            let shown = link_display(link);
            match resolution {
                Resolution::Resolved(t) => {
                    let target = &ws.notes()[*t];
                    let mut text = format!("\u{2192} {}", shown);
                    if !ws.fragment_exists(*t, link) {
                        text.push_str("  (heading not found)");
                    }
                    links.push(LinkItem {
                        text: text.as_str().into(),
                        path: target.path.as_str().into(),
                        tint: col(colors.link),
                        header: false,
                    });
                }
                Resolution::Ambiguous(c) => {
                    let text = format!("\u{26a0} ambiguous ({} matches): {}", c.len(), shown);
                    links.push(LinkItem {
                        text: text.as_str().into(),
                        path: "".into(),
                        tint: col(colors.warning),
                        header: false,
                    });
                }
                Resolution::Unresolved => {
                    let text = format!("\u{2717} not found: {}", shown);
                    links.push(LinkItem {
                        text: text.as_str().into(),
                        path: "".into(),
                        tint: col(colors.error),
                        header: false,
                    });
                }
            }
        }

        let back = ws.backlinks(idx);
        links.push(header(format!("Backlinks ({})", back.len()), colors));
        for b in back {
            let from = &ws.notes()[b.from];
            let text = format!("\u{2190} {} \u{2014} {}", from.title, b.context);
            links.push(LinkItem {
                text: text.as_str().into(),
                path: from.path.as_str().into(),
                tint: col(colors.text),
                header: false,
            });
        }
    }
    w.set_link_items(model(links));
}

fn header(text: String, colors: &Colors) -> LinkItem {
    LinkItem {
        text: text.as_str().into(),
        path: "".into(),
        tint: col(colors.text),
        header: true,
    }
}

fn link_display(link: &Link) -> String {
    let base = match (&link.label, link.target.is_empty()) {
        (Some(label), _) => label.clone(),
        (None, false) => link.target.clone(),
        (None, true) => "(this note)".to_string(),
    };
    let frag = match &link.fragment {
        Fragment::None => String::new(),
        Fragment::Heading(h) => format!(" \u{00a7} {}", h),
        Fragment::Block(b) => format!(" \u{00a7} block {}", b),
    };
    let kind = match link.kind {
        LinkKind::Wiki | LinkKind::External => "",
        LinkKind::Markdown => " (link)",
    };
    format!("{}{}{}", base, frag, kind)
}

/// Maps a UI-independent [`Card`] onto Slint styling.
fn to_item(card: &Card, c: &Colors, scale: f32, links: Vec<PreviewLink>) -> PreviewItem {
    let (fill, accent, font, bold, mono) = match &card.kind {
        CardKind::Heading(level) => {
            let size = match level {
                1 => 28.0,
                2 => 23.0,
                3 => 19.0,
                _ => 16.0,
            };
            (transparent(), transparent(), size, true, false)
        }
        CardKind::Paragraph => (transparent(), transparent(), 15.0, false, false),
        CardKind::Code => (col(c.code_fill), col(c.code_accent), 13.0, false, true),
        CardKind::Meta => (col(c.meta_fill), col(c.meta_accent), 13.0, false, false),
        CardKind::Callout(kind) => {
            let (fill, accent) = c.callout(kind);
            (col(fill), col(accent), 15.0, false, false)
        }
        CardKind::Details => (col(c.details_fill), col(c.details_accent), 15.0, false, false),
        CardKind::TabGroup | CardKind::Tab => {
            (col(c.tab_fill), col(c.tab_accent), 15.0, false, false)
        }
        CardKind::Unknown => (col(c.unknown_fill), col(c.unknown_accent), 13.0, false, true),
    };
    PreviewItem {
        title: card.title.as_str().into(),
        body: card.body.as_str().into(),
        indent: card.depth as f32 * 18.0 * scale,
        fill,
        accent,
        font_size: font * scale,
        bold,
        mono,
        links: model(links),
    }
}
