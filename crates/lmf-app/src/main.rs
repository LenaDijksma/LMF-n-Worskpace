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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use lmf_core::links::{Fragment, Link, LinkKind};
use lmf_core::preview::{build_cards, cards_from_text, Align, Card, CardKind, TableData};
use lmf_core::media::{classify_target, encode_dest, extract_media, MediaTarget};
use lmf_core::richtext::{
    decode_file_href, decode_href, display_markdown, encode_href, escape, LinkTarget,
};
use lmf_core::{check, parse, LineIndex, Severity};
use lmf_workspace::{assets, scan, NoteSource, Resolution, Workspace};
use slint::{Color, ComponentHandle, Model, ModelRc, StyledText, VecModel};

use settings::{step_scale, Settings};
use theme::{Colors, Rgb, ThemeKind};

slint::include_modules!();

const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(2);
/// While typing, the problem list is refreshed this long after the last keystroke. (The preview is
/// not rendered at all in Edit view.)
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(400);

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
    /// The preview list shown by the window. Updated row by row, never replaced wholesale.
    preview_model: Rc<VecModel<PreviewItem>>,
    /// The cards the preview currently shows (for diffing against the next version).
    last_cards: RefCell<Vec<Card>>,
    /// What the shown rows were rendered with: (theme, text scale bits, workspace revision, note).
    last_style: Cell<Option<(ThemeKind, u32, u64, usize)>>,
    /// Incremented whenever the workspace index changes, so link states are re-rendered.
    revision: u64,
    /// Decoded images by file, with the file's modification time and size when they were loaded.
    image_cache: RefCell<HashMap<PathBuf, (SystemTime, u64, slint::Image)>>,
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

    let preview_model: Rc<VecModel<PreviewItem>> = Rc::new(VecModel::default());
    window.set_preview_items(ModelRc::from(preview_model.clone()));
    let state = Rc::new(RefCell::new(App {
        settings,
        root: None,
        workspace: None,
        path: None,
        dirty: false,
        disk_stamp: None,
        query: String::new(),
        preview_model: preview_model.clone(),
        last_cards: RefCell::new(Vec::new()),
        last_style: Cell::new(None),
        revision: 0,
        image_cache: RefCell::new(HashMap::new()),
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
    // Typing only marks the document dirty; the problem list is refreshed once typing pauses.
    // Edit view renders nothing else; the preview is built when switching to Preview view.
    let preview_timer = slint::Timer::default();
    window.on_text_edited({
        let weak = window.as_weak();
        let state = state.clone();
        move |_text| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            st.dirty = true;
            let message = if st.path.is_some() {
                "Unsaved changes (autosave on)"
            } else {
                "Unsaved new document"
            };
            w.set_save_status(message.into());
            drop(st);

            let weak = weak.clone();
            let state = state.clone();
            preview_timer.start(slint::TimerMode::SingleShot, PREVIEW_DEBOUNCE, move || {
                let Some(w) = weak.upgrade() else { return };
                let Ok(st) = state.try_borrow() else { return };
                let text = w.get_editor_text();
                refresh(&w, text.as_str(), &st);
            });
        }
    });

    // --- files ----------------------------------------------------------------------------
    window.on_toggle_mode({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(st) = state.try_borrow() else { return };
            let to_preview = !w.get_preview_mode();
            w.set_preview_mode(to_preview);
            if to_preview {
                // The only moment the preview is rendered after editing.
                let text = w.get_editor_text();
                refresh(&w, text.as_str(), &st);
            }
        }
    });

    window.on_attach_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            let Some(root) = st.root.clone() else {
                w.set_save_status("Open a folder first, so attachments have somewhere to live".into());
                return;
            };
            let Some(source) = rfd::FileDialog::new().pick_file() else {
                return;
            };
            match assets::attach_file(&root, &source) {
                Ok(asset_rel) => {
                    let note_dir = current_rel(&st)
                        .map(|r| assets::parent_dir(&r))
                        .unwrap_or_default();
                    let rel = assets::relative_path(&note_dir, &asset_rel);
                    let name = asset_rel.rsplit('/').next().unwrap_or(&asset_rel).to_string();
                    let stem = name
                        .rsplit_once('.')
                        .map(|(s, _)| s.to_string())
                        .unwrap_or_else(|| name.clone());
                    let is_image = assets::extension(&name)
                        .map(|e| assets::is_image_ext(&e))
                        .unwrap_or(false);
                    let reference = if is_image {
                        format!("![{}]({})", stem, encode_dest(&rel))
                    } else {
                        format!("[{}]({})", name, encode_dest(&rel))
                    };
                    let text = w.get_editor_text();
                    let separator = if text.is_empty() || text.ends_with("\n\n") {
                        ""
                    } else if text.ends_with('\n') {
                        "\n"
                    } else {
                        "\n\n"
                    };
                    let new_text = format!("{}{}{}\n", text, separator, reference);
                    w.set_editor_text(new_text.as_str().into());
                    st.dirty = true;
                    let message = format!("Attached {} (copied into assets/)", name);
                    refresh(&w, &new_text, &st);
                    w.set_save_status(message.as_str().into());
                }
                Err(e) => w.set_save_status(e.as_str().into()),
            }
        }
    });

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

    window.on_rich_link_clicked({
        let weak = window.as_weak();
        let state = state.clone();
        move |url| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(mut st) = state.try_borrow_mut() else { return };
            if let Some(path) = decode_href(url.as_str()) {
                go_to(&w, &mut st, &path);
            } else if let Some(target) = decode_file_href(url.as_str()) {
                open_attachment(&w, &st, &target);
            } else {
                open_external(&w, url.as_str());
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

/// Opens an attachment (a link to a local file) after vetting: the path must stay inside the
/// workspace, the type must be on the safe list, and the user must confirm.
fn open_attachment(w: &MainWindow, app: &App, target: &str) {
    let Some(root) = &app.root else {
        w.set_save_status("Open a folder first to open attachments".into());
        return;
    };
    let note_rel = current_rel(app).unwrap_or_default();
    let path = match assets::resolve(root, &note_rel, target) {
        Ok(p) => p,
        Err(e) => {
            let message = format!("Not opened: {}", e);
            w.set_save_status(message.as_str().into());
            return;
        }
    };
    let ext = assets::extension(target).unwrap_or_default();
    if !assets::is_openable_ext(&ext) {
        let message = format!("Not opened: .{} files are not on the safe list", ext);
        w.set_save_status(message.as_str().into());
        return;
    }
    let question = format!(
        "This opens the file with your system's default program:\n\n{}",
        target
    );
    if !confirm("Open attachment?", &question) {
        return;
    }
    if let Err(e) = external::open_path(&path) {
        w.set_save_status(e.as_str().into());
    }
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
            app.revision += 1;
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
                app.revision += 1;
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
    theme.set_border(col(c.border));
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
    let mut diags = check(&doc);
    // Images and attachments: do the files exist, and are they allowed?
    if let Some(root) = &app.root {
        let note_rel = current_rel(app).unwrap_or_default();
        let refs = extract_media(&doc);
        diags.extend(assets::check_assets(root, &note_rel, &refs));
        lmf_core::diagnostics::sort_diagnostics(&mut diags);
    }
    let index = LineIndex::new(&doc.source);

    // The preview is only rendered in Preview view; Edit view does no rendering work.
    if w.get_preview_mode() {
        let from = current_note_index(app);
        let chain: Vec<usize> = from.into_iter().collect();
        let cards = expand_includes(build_cards(&doc), app, &chain);
        update_preview(app, cards, colors, scale, from.unwrap_or(0));
    }

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

/// Whether a card can be left untouched. Image cards are always re-rendered, because the file
/// behind them can change without the card changing (the image cache keeps this cheap).
fn same_card(a: &Card, b: &Card) -> bool {
    a == b && !matches!(a.kind, CardKind::Image { .. })
}

/// Brings the preview list in line with `cards`. Rows that did not change are left alone (their
/// rich text is not re-parsed and their widgets are not rebuilt); only the changed stretch in the
/// middle is updated. Everything is rebuilt when the theme, text size, workspace or open note
/// changed, since those affect how unchanged cards look.
fn update_preview(app: &App, cards: Vec<Card>, colors: &Colors, scale: f32, from: usize) {
    let style = (app.settings.theme, scale.to_bits(), app.revision, from);
    let model = &app.preview_model;
    let mut renderer = Renderer { app, from };
    let mut last = app.last_cards.borrow_mut();

    if app.last_style.get() != Some(style) {
        let items: Vec<PreviewItem> = cards
            .iter()
            .map(|card| to_item(card, colors, scale, &mut renderer))
            .collect();
        model.set_vec(items);
    } else {
        let old_len = last.len();
        let new_len = cards.len();
        let mut head = 0usize;
        while head < old_len.min(new_len) && same_card(&last[head], &cards[head]) {
            head += 1;
        }
        let mut tail = 0usize;
        while tail < (old_len - head).min(new_len - head)
            && same_card(&last[old_len - 1 - tail], &cards[new_len - 1 - tail])
        {
            tail += 1;
        }
        let removed = old_len - head - tail;
        let added = new_len - head - tail;
        let common = removed.min(added);
        for k in 0..common {
            let item = to_item(&cards[head + k], colors, scale, &mut renderer);
            model.set_row_data(head + k, item);
        }
        for _ in common..removed {
            let _ = model.remove(head + common);
        }
        for k in common..added {
            let item = to_item(&cards[head + k], colors, scale, &mut renderer);
            model.insert(head + k, item);
        }
    }
    *last = cards;
    app.last_style.set(Some(style));
}

/// Replaces `lmf:include` cards with the included content (recursively), or with a warning card
/// explaining why it could not be expanded. The author's fallback text is always kept visible in
/// the warning case, so the document still reads sensibly.
fn expand_includes(cards: Vec<Card>, app: &App, chain: &[usize]) -> Vec<Card> {
    let mut out = Vec::new();
    for card in cards {
        let include = match &card.kind {
            CardKind::Include {
                document,
                section,
                block,
            } => Some((document.clone(), section.clone(), block.clone())),
            _ => None,
        };
        let Some((document, section, block)) = include else {
            out.push(card);
            continue;
        };
        let Some(ws) = &app.workspace else {
            out.push(include_problem(&card, "open a folder to expand includes"));
            continue;
        };
        match ws.expand_include(chain, &document, section.as_deref(), block.as_deref()) {
            Ok(expansion) => {
                out.push(Card {
                    kind: card.kind.clone(),
                    title: format!("\u{21b3} Included from {} ({})", expansion.title, expansion.path),
                    body: String::new(),
                    depth: card.depth,
                });
                let mut next = chain.to_vec();
                next.push(expansion.note);
                let inner = cards_from_text(&expansion.text, card.depth + 1);
                out.extend(expand_includes(inner, app, &next));
            }
            Err(e) => out.push(include_problem(&card, &e.to_string())),
        }
    }
    out
}

fn include_problem(card: &Card, message: &str) -> Card {
    Card {
        kind: CardKind::Callout("warning".to_string()),
        title: format!("Include problem: {}", message),
        body: card.body.clone(),
        depth: card.depth,
    }
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

/// Turns Markdown text into Slint rich text. Internal links get self-describing `lmf:<path>`
/// destinations; unresolvable ones are struck through; HTML is neutralised (see
/// `lmf_core::richtext`). If the widget rejects the Markdown, the original text is shown plainly.
struct Renderer<'a> {
    app: &'a App,
    /// Note that contains the text being rendered (for resolving relative names).
    from: usize,
}

impl Renderer<'_> {
    fn resolve(&mut self, link: &Link) -> LinkTarget {
        let Some(ws) = &self.app.workspace else {
            return LinkTarget::Unresolved;
        };
        match ws.resolve(self.from, link) {
            Resolution::Resolved(t) => LinkTarget::Href(encode_href(&ws.notes()[t].path)),
            _ => LinkTarget::Unresolved,
        }
    }

    /// Loads a local image if policy allows (inside the workspace, png/jpg/gif, at most 4 MiB and
    /// 16 megapixels). Remote and unsupported addresses are never loaded. On failure returns a
    /// Markdown explanation for the preview.
    fn load_image(&mut self, src: &str) -> Result<slint::Image, String> {
        match classify_target(src) {
            MediaTarget::Remote(url) => Err(format!(
                "Remote images are never loaded (nothing is fetched from the web): {}",
                escape(&url)
            )),
            MediaTarget::Unsupported(address) => {
                Err(format!("Unsupported image address: {}", escape(&address)))
            }
            MediaTarget::Local(path) => {
                let Some(root) = &self.app.root else {
                    return Err(format!(
                        "Open a folder to show local images: {}",
                        escape(&path)
                    ));
                };
                let note_rel = current_rel(self.app).unwrap_or_default();
                let full = assets::resolve(root, &note_rel, &path)
                    .map_err(|e| format!("{}: {}", escape(&path), e))?;
                let file_stamp = stamp(&full);
                {
                    let cache = self.app.image_cache.borrow();
                    if let (Some(s), Some((t, l, img))) = (file_stamp, cache.get(&full)) {
                        if (*t, *l) == s {
                            return Ok(img.clone());
                        }
                    }
                }
                assets::check_image(&full).map_err(|e| format!("{}: {}", escape(&path), e))?;
                let img = slint::Image::load_from_path(&full)
                    .map_err(|_| format!("{}: could not decode the image", escape(&path)))?;
                if let Some((t, l)) = file_stamp {
                    let mut cache = self.app.image_cache.borrow_mut();
                    if cache.len() >= 32 {
                        cache.clear();
                    }
                    cache.insert(full, (t, l, img.clone()));
                }
                Ok(img)
            }
        }
    }

    fn rich(&mut self, markdown: &str) -> StyledText {
        if markdown.trim().is_empty() {
            return StyledText::from_plain_text("");
        }
        let prepared = {
            let mut resolver = |link: &Link| self.resolve(link);
            display_markdown(markdown, &mut resolver)
        };
        StyledText::from_markdown(&prepared)
            .unwrap_or_else(|_| StyledText::from_plain_text(markdown))
    }
}

fn table_row(r: &mut Renderer<'_>, cells: &[String], align: &[Align], header: bool) -> TableRow {
    let items: Vec<TableCell> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let markdown = if header && !cell.is_empty() {
                format!("**{}**", cell)
            } else {
                cell.clone()
            };
            let code = match align.get(i) {
                Some(Align::Center) => 1,
                Some(Align::Right) => 2,
                _ => 0,
            };
            TableCell {
                rich: r.rich(&markdown),
                align: code,
            }
        })
        .collect();
    TableRow {
        cells: model(items),
        header,
    }
}

fn table_rows(r: &mut Renderer<'_>, t: &TableData) -> Vec<TableRow> {
    let mut rows = vec![table_row(r, &t.header, &t.align, true)];
    for row in &t.rows {
        rows.push(table_row(r, row, &t.align, false));
    }
    rows
}

/// Maps a UI-independent [`Card`] onto Slint styling and rich text.
fn to_item(card: &Card, c: &Colors, scale: f32, r: &mut Renderer<'_>) -> PreviewItem {
    let mut fill = transparent();
    let mut accent = transparent();
    let mut font = 15.0f32;
    let mut mono = false;
    let mut rich_source: Option<String> = None;
    let mut rule = false;
    let mut rows: Vec<TableRow> = Vec::new();
    let mut has_image = false;
    let mut image = slint::Image::default();
    let mut title = card.title.clone();

    match &card.kind {
        CardKind::Heading(level) => {
            font = match level {
                1 => 28.0,
                2 => 23.0,
                3 => 19.0,
                _ => 16.0,
            };
            let text = card.body.trim();
            if !text.is_empty() {
                rich_source = Some(format!("**{}**", text));
            }
        }
        CardKind::Paragraph => rich_source = Some(card.body.clone()),
        CardKind::Image { src } => match r.load_image(src) {
            Ok(img) => {
                has_image = true;
                image = img;
            }
            Err(message) => {
                let (f, a) = c.callout("warning");
                fill = col(f);
                accent = col(a);
                title = "Image not shown".to_string();
                rich_source = Some(message);
            }
        },
        CardKind::Code | CardKind::Math | CardKind::Diagram => {
            fill = col(c.code_fill);
            accent = col(c.code_accent);
            font = 13.0;
            mono = true;
        }
        CardKind::Meta => {
            fill = col(c.meta_fill);
            accent = col(c.meta_accent);
            font = 13.0;
        }
        CardKind::Callout(kind) => {
            let (f, a) = c.callout(kind);
            fill = col(f);
            accent = col(a);
            rich_source = Some(card.body.clone());
        }
        CardKind::Details => {
            fill = col(c.details_fill);
            accent = col(c.details_accent);
            rich_source = Some(card.body.clone());
        }
        CardKind::TabGroup | CardKind::Tab | CardKind::Include { .. } => {
            fill = col(c.tab_fill);
            accent = col(c.tab_accent);
            rich_source = Some(card.body.clone());
        }
        CardKind::Quote => {
            fill = col(c.details_fill);
            accent = col(c.details_accent);
            rich_source = Some(card.body.clone());
        }
        CardKind::Rule => {
            accent = col(c.border);
            rule = true;
        }
        CardKind::Table(t) => {
            rows = table_rows(r, t);
        }
        CardKind::Unknown => {
            fill = col(c.unknown_fill);
            accent = col(c.unknown_accent);
            font = 13.0;
            mono = true;
        }
    }

    let (is_rich, rich) = match rich_source {
        Some(markdown) if !markdown.trim().is_empty() => (true, r.rich(&markdown)),
        _ => (false, StyledText::from_plain_text("")),
    };
    // Plain text is only used when there is no rich text.
    let body = if is_rich { String::new() } else { card.body.clone() };

    PreviewItem {
        title: title.as_str().into(),
        body: body.as_str().into(),
        indent: card.depth as f32 * 18.0 * scale,
        fill,
        accent,
        font_size: font * scale,
        mono,
        is_rich,
        rich,
        rule,
        has_image,
        image,
        rows: model(rows),
    }
}
