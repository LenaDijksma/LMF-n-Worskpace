//! Tasks across the whole workspace: listing, queries, identity checks and safe file writes.
//!
//! A task lives in exactly one note (the one whose source contains its line). Everything here
//! *refers* to tasks; nothing copies them, so a task has one canonical identity however many
//! views (list, board, agenda) show it.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use lmf_core::diagnostics::{codes, Diagnostic};
use lmf_core::taskquery::{Ctx, TaskQuery, TaskView};
use lmf_core::tasks::Task;

use crate::assets;
use crate::index::Workspace;

/// A task together with where it lives.
#[derive(Debug, Clone, Copy)]
pub struct TaskHit<'a> {
    /// Index of the note in [`Workspace::notes`].
    pub note: usize,
    /// Index of the task in that note's `tasks`.
    pub index: usize,
    pub task: &'a Task,
    pub path: &'a str,
    /// Some task listed in `[after: …]` is not done yet.
    pub blocked: bool,
}

impl<'a> TaskHit<'a> {
    pub fn view(&self) -> TaskView<'a> {
        TaskView {
            task: self.task,
            note_path: self.path,
            blocked: self.blocked,
        }
    }
}

/// The same task id used in more than one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateId {
    pub id: String,
    /// `path:line` of every use.
    pub places: Vec<String>,
}

impl Workspace {
    /// Every task of every note, in note order then source order.
    pub fn tasks(&self) -> Vec<TaskHit<'_>> {
        // id -> done, for dependency checks (the first task with an id wins).
        let mut done_by_id: HashMap<&str, bool> = HashMap::new();
        for note in self.notes() {
            for t in &note.tasks {
                if let Some(id) = &t.id {
                    done_by_id.entry(id.as_str()).or_insert(t.done);
                }
            }
        }
        let mut out = Vec::new();
        for (ni, note) in self.notes().iter().enumerate() {
            for (ti, t) in note.tasks.iter().enumerate() {
                let blocked = t
                    .after
                    .iter()
                    .any(|dep| matches!(done_by_id.get(dep.as_str()), Some(false)));
                out.push(TaskHit {
                    note: ni,
                    index: ti,
                    task: t,
                    path: note.path.as_str(),
                    blocked,
                });
            }
        }
        out
    }

    /// Tasks matching `query`, sorted by the query's order.
    pub fn query_tasks(&self, query: &TaskQuery, ctx: &Ctx) -> Vec<TaskHit<'_>> {
        let mut hits: Vec<TaskHit<'_>> = self
            .tasks()
            .into_iter()
            .filter(|h| query.matches(&h.view(), ctx))
            .collect();
        hits.sort_by(|a, b| query.compare(&a.view(), &b.view()));
        hits
    }

    /// The first task with this stable id.
    pub fn task_by_id(&self, id: &str) -> Option<TaskHit<'_>> {
        self.tasks().into_iter().find(|h| h.task.id.as_deref() == Some(id))
    }

    /// Ids used by more than one task anywhere in the workspace.
    pub fn duplicate_task_ids(&self) -> Vec<DuplicateId> {
        let mut places: HashMap<&str, Vec<String>> = HashMap::new();
        for note in self.notes() {
            for t in &note.tasks {
                if let Some(id) = &t.id {
                    places
                        .entry(id.as_str())
                        .or_default()
                        .push(format!("{}:{}", note.path, t.line));
                }
            }
        }
        let mut dups: Vec<DuplicateId> = places
            .into_iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(id, v)| DuplicateId {
                id: id.to_string(),
                places: v,
            })
            .collect();
        dups.sort_by(|a, b| a.id.cmp(&b.id));
        dups
    }

    /// Warnings (`LMF022`) for tasks of `note` whose id is also used by a task in another note.
    /// (A repeated id inside one note is `LMF005`, reported by `lmf_core::check`.)
    pub fn duplicate_id_diagnostics(&self, note: usize) -> Vec<Diagnostic> {
        let mut uses: HashMap<&str, Vec<(usize, usize, &str)>> = HashMap::new();
        for (ni, n) in self.notes().iter().enumerate() {
            for t in &n.tasks {
                if let Some(id) = &t.id {
                    uses.entry(id.as_str()).or_default().push((ni, t.line, n.path.as_str()));
                }
            }
        }
        let mut out = Vec::new();
        let Some(n) = self.notes().get(note) else {
            return out;
        };
        for t in &n.tasks {
            let Some(id) = &t.id else { continue };
            let others: Vec<String> = uses
                .get(id.as_str())
                .map(|v| {
                    v.iter()
                        .filter(|(ni, _, _)| *ni != note)
                        .map(|(_, line, path)| format!("{}:{}", path, line))
                        .collect()
                })
                .unwrap_or_default();
            if !others.is_empty() {
                out.push(Diagnostic::warning(
                    codes::DUPLICATE_TASK_ID,
                    format!("task id `{}` is also used in {}", id, others.join(", ")),
                    t.span,
                ));
            }
        }
        out
    }

    /// All block ids and task ids in the workspace (for generating ids that cannot collide).
    pub fn taken_ids(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        for note in self.notes() {
            for id in &note.block_ids {
                set.insert(id.clone());
            }
            for t in &note.tasks {
                if let Some(id) = &t.id {
                    set.insert(id.clone());
                }
            }
        }
        set
    }
}

/// Replaces the content of a note file, but only if it still contains `expected` (nobody changed
/// it since it was read). The new content is written to a temporary file next to the note and
/// renamed over it, so a crash cannot leave a half-written note.
pub fn write_note_atomic(root: &Path, rel: &str, expected: &str, new_text: &str) -> Result<(), String> {
    // `resolve` confines the path to the workspace folder (no `..`, no symlink escapes).
    let full = assets::resolve(root, "", rel).map_err(|e| format!("{}: {}", rel, e))?;
    let current = fs::read_to_string(&full).map_err(|e| format!("cannot read {}: {}", rel, e))?;
    if current != expected {
        return Err(format!("{} changed on disk since it was read; nothing was written", rel));
    }
    let name = full
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note".to_string());
    let tmp = full.with_file_name(format!(".{}.lovely-tmp", name));
    fs::write(&tmp, new_text).map_err(|e| format!("cannot write {}: {}", rel, e))?;
    if let Ok(meta) = fs::metadata(&full) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = fs::rename(&tmp, &full) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot replace {}: {}", rel, e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::NoteSource;
    use lmf_core::dates::Date;
    use lmf_core::taskedit::{apply_edits, assign_ids, move_to_column};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("lmf-tasks-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ws(items: &[(&str, &str)]) -> Workspace {
        Workspace::build(
            items
                .iter()
                .map(|(p, t)| NoteSource {
                    path: p.to_string(),
                    text: t.to_string(),
                })
                .collect(),
        )
    }

    fn today() -> Ctx {
        Ctx {
            today: Date::parse("2026-11-03").unwrap(),
        }
    }

    fn sample() -> Workspace {
        ws(&[
            ("a.md", "- [ ] Write parser [due: 2026-11-03] ^t-parser\n- [ ] Ship it [after: t-parser]\n"),
            ("projects/b.md", "- [x] Choose name ^t-name\n- [ ] Draw logo [status: active] #design\n"),
        ])
    }

    #[test]
    fn lists_tasks_across_notes() {
        let w = sample();
        let all = w.tasks();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].path, "a.md");
        assert_eq!(all[2].path, "projects/b.md");
    }

    #[test]
    fn queries_run_across_notes() {
        let w = sample();
        let hits = w.query_tasks(&TaskQuery::parse("note:projects/ is:open").unwrap(), &today());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].task.title, "Draw logo #design");
        let due = w.query_tasks(&TaskQuery::parse("due:today").unwrap(), &today());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].task.id.as_deref(), Some("t-parser"));
    }

    #[test]
    fn dependencies_block_until_done() {
        let w = sample();
        let blocked = w.query_tasks(&TaskQuery::parse("is:blocked").unwrap(), &today());
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].task.title, "Ship it");
        // Once the prerequisite is done, nothing is blocked.
        let w2 = ws(&[("a.md", "- [x] Write parser ^t-parser\n- [ ] Ship it [after: t-parser]\n")]);
        assert!(w2.query_tasks(&TaskQuery::parse("is:blocked").unwrap(), &today()).is_empty());
        // An unknown dependency does not block.
        let w3 = ws(&[("a.md", "- [ ] Ship it [after: t-ghost]\n")]);
        assert!(w3.query_tasks(&TaskQuery::parse("is:blocked").unwrap(), &today()).is_empty());
    }

    #[test]
    fn identity_lookup_and_duplicates() {
        let w = sample();
        assert_eq!(w.task_by_id("t-name").unwrap().path, "projects/b.md");
        assert!(w.task_by_id("t-nope").is_none());
        assert!(w.duplicate_task_ids().is_empty());
        let dup = ws(&[("a.md", "- [ ] x ^t-same\n"), ("b.md", "- [ ] y ^t-same\n")]);
        let d = dup.duplicate_task_ids();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, "t-same");
        assert_eq!(d[0].places, vec!["a.md:1", "b.md:1"]);
    }

    #[test]
    fn duplicate_ids_across_notes_are_warned_about() {
        let w = ws(&[("a.md", "- [ ] x ^t-same\n"), ("b.md", "- [ ] y ^t-same\n- [ ] z ^t-other\n")]);
        let a = w.duplicate_id_diagnostics(0);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].code, "LMF022");
        assert!(a[0].message.contains("b.md:1"), "{}", a[0].message);
        assert_eq!(w.duplicate_id_diagnostics(1).len(), 1);
        // A repeat inside one note is LMF005's business, not this check's.
        let one = ws(&[("a.md", "- [ ] x ^t-same\n- [ ] y ^t-same\n")]);
        assert!(one.duplicate_id_diagnostics(0).is_empty());
        assert!(w.duplicate_id_diagnostics(99).is_empty());
    }

    #[test]
    fn included_content_is_never_a_second_task() {
        let w = ws(&[
            ("a.md", "- [ ] Real task ^t-real\n"),
            ("b.md", ":::lmf:include {document=\"a\"}\n:::\n"),
        ]);
        assert_eq!(w.tasks().len(), 1);
    }

    #[test]
    fn taken_ids_cover_block_ids_and_tasks() {
        let w = ws(&[("a.md", "- [ ] x ^t-one\n\nPara ^para-id\n")]);
        let taken = w.taken_ids();
        assert!(taken.contains("t-one") && taken.contains("para-id"));
    }

    #[test]
    fn writing_is_atomic_and_detects_changes() {
        let root = temp_dir();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/n.md"), "- [ ] a\n").unwrap();

        write_note_atomic(&root, "sub/n.md", "- [ ] a\n", "- [x] a\n").unwrap();
        assert_eq!(fs::read_to_string(root.join("sub/n.md")).unwrap(), "- [x] a\n");
        // No temporary files are left behind.
        let leftovers: Vec<_> = fs::read_dir(root.join("sub")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers.len(), 1);

        // Stale expectation: the file is not touched.
        let err = write_note_atomic(&root, "sub/n.md", "- [ ] a\n", "- [ ] SOMETHING ELSE\n").unwrap_err();
        assert!(err.contains("changed on disk"), "{}", err);
        assert_eq!(fs::read_to_string(root.join("sub/n.md")).unwrap(), "- [x] a\n");

        // Paths cannot leave the workspace.
        assert!(write_note_atomic(&root, "../evil.md", "", "x").is_err());
        assert!(write_note_atomic(&root, "missing.md", "", "x").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn moving_a_card_end_to_end_keeps_identity_and_content() {
        let root = temp_dir();
        let text = "# Board\n\n- [ ] Draw logo ^t-logo\n- [ ] Other\n";
        fs::write(root.join("b.md"), text).unwrap();

        let mut w = Workspace::build(vec![NoteSource { path: "b.md".into(), text: text.into() }]);
        let hit = w.task_by_id("t-logo").unwrap();
        let task = hit.task.clone();
        let today = Date::parse("2026-11-03").unwrap();
        let edits = move_to_column(text, &task, "active", "done", "todo", today).unwrap();
        let new_text = apply_edits(text, &edits).unwrap();
        write_note_atomic(&root, "b.md", text, &new_text).unwrap();
        w.upsert_note(NoteSource { path: "b.md".into(), text: new_text.clone() });

        // Same identity, same count, status changed, nothing else.
        assert_eq!(w.tasks().len(), 2);
        let moved = w.task_by_id("t-logo").unwrap();
        assert_eq!(moved.task.status.as_deref(), Some("active"));
        assert_eq!(moved.task.title, "Draw logo");
        assert_eq!(new_text, "# Board\n\n- [ ] Draw logo [status: active] ^t-logo\n- [ ] Other\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn assigning_ids_across_the_workspace_avoids_collisions() {
        let w = ws(&[("a.md", "- [ ] one\n- [ ] two ^t-have\n")]);
        let note = &w.notes()[0];
        let edits = assign_ids(&note.text, &note.tasks, &w.taken_ids(), &note.path);
        assert_eq!(edits.len(), 1);
        let out = apply_edits(&note.text, &edits).unwrap();
        let w2 = ws(&[("a.md", out.as_str())]);
        assert_eq!(w2.tasks().iter().filter(|h| h.task.id.is_some()).count(), 2);
        assert!(w2.duplicate_task_ids().is_empty());
    }
}
