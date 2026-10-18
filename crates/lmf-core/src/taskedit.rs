//! Safe, minimal edits to task lines.
//!
//! Every function here is *pure*: it takes the current text of a note plus a [`Task`] parsed from
//! it and returns [`TextEdit`]s. Nothing touches the filesystem. A task whose line no longer
//! matches the text it was parsed from is rejected as [`EditError::Stale`], so an edit can never
//! land on the wrong line.
//!
//! What an edit may change: the checkbox character, the task's own `[key: value]` fields, a
//! trailing id marker, and (for recurring tasks) one inserted line. Every other byte of the note
//! stays exactly as it was; moving a task between columns never duplicates or deletes content.

use std::collections::HashSet;
use std::fmt;

use crate::dates::{Date, TaskDate};
use crate::structure::trailing_marker;
use crate::tasks::{find_fields, Task};

/// Replace `start..end` of the text with `replacement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The text no longer contains the task line this edit was computed for.
    Stale,
    Overlap,
    InvalidRange,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Stale => write!(f, "the note changed since the task was read"),
            EditError::Overlap => write!(f, "overlapping edits"),
            EditError::InvalidRange => write!(f, "edit range is invalid"),
        }
    }
}

/// Applies non-overlapping edits to `text`.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.start, e.end));
    let mut out = String::with_capacity(text.len() + 32);
    let mut pos = 0usize;
    for e in sorted {
        if e.start < pos {
            return Err(EditError::Overlap);
        }
        if e.start > e.end
            || e.end > text.len()
            || !text.is_char_boundary(e.start)
            || !text.is_char_boundary(e.end)
        {
            return Err(EditError::InvalidRange);
        }
        out.push_str(&text[pos..e.start]);
        out.push_str(&e.replacement);
        pos = e.end;
    }
    out.push_str(&text[pos..]);
    Ok(out)
}

fn verify_fresh(text: &str, task: &Task) -> Result<(), EditError> {
    if text.get(task.span.start..task.span.end) == Some(task.raw.as_str()) {
        Ok(())
    } else {
        Err(EditError::Stale)
    }
}

/// Edits one task line as text.
struct LineEditor {
    raw: String,
    rest_offset: usize,
    checkbox_off: usize,
}

impl LineEditor {
    fn new(task: &Task) -> Self {
        LineEditor {
            raw: task.raw.clone(),
            rest_offset: task.rest_offset,
            checkbox_off: task.checkbox.start - task.span.start,
        }
    }

    fn set_checkbox(&mut self, ch: char) {
        let off = self.checkbox_off;
        if self.raw.is_char_boundary(off) && self.raw.is_char_boundary(off + 1) {
            self.raw = format!("{}{}{}", &self.raw[..off], ch, &self.raw[off + 1..]);
        }
    }

    /// Removes every `[key: …]` field (and one adjacent space).
    fn remove_field(&mut self, key: &str) {
        loop {
            let rest = &self.raw[self.rest_offset..];
            let Some(seg) = find_fields(rest).into_iter().find(|s| s.key == key) else {
                return;
            };
            let mut start = self.rest_offset + seg.start;
            let end = self.rest_offset + seg.end;
            if self.raw[..start].ends_with(' ') {
                start -= 1;
            }
            self.raw = format!("{}{}", &self.raw[..start], &self.raw[end..]);
        }
    }

    /// Where a new field goes: before a trailing id marker, else at the end of the line.
    fn insertion_point(&self) -> usize {
        let rest = &self.raw[self.rest_offset..];
        match trailing_marker(rest) {
            Some((caret, _)) => self.rest_offset + rest[..caret].trim_end().len(),
            None => self.raw.trim_end().len(),
        }
    }

    fn upsert_field(&mut self, key: &str, value: &str) {
        let field = format!("[{}: {}]", key, value);
        let rest = &self.raw[self.rest_offset..];
        if let Some(seg) = find_fields(rest).into_iter().find(|s| s.key == key) {
            let start = self.rest_offset + seg.start;
            let end = self.rest_offset + seg.end;
            self.raw = format!("{}{}{}", &self.raw[..start], field, &self.raw[end..]);
        } else {
            let at = self.insertion_point();
            self.raw = format!("{} {}{}", &self.raw[..at], field, &self.raw[at..]);
        }
    }

    fn strip_id(&mut self) {
        let rest = &self.raw[self.rest_offset..];
        if let Some((caret, _)) = trailing_marker(rest) {
            let cut = self.rest_offset + rest[..caret].trim_end().len();
            self.raw.truncate(cut);
        }
    }
}

enum StatusChange {
    Keep,
    Remove,
    Set(String),
}

/// Marks a task done (with a completion date) or open again. Completing a recurring task also
/// inserts the next occurrence on the line below (see [`crate::tasks::Repeat`]).
pub fn set_done(text: &str, task: &Task, done: bool, today: Date) -> Result<Vec<TextEdit>, EditError> {
    verify_fresh(text, task)?;
    Ok(finish(text, task, today, done, StatusChange::Keep))
}

/// Moves a task to a board column. The `done_column` completes it, the `todo_column` clears its
/// status, any other column sets `[status: column]` (and reopens a done task).
pub fn move_to_column(
    text: &str,
    task: &Task,
    column: &str,
    done_column: &str,
    todo_column: &str,
    today: Date,
) -> Result<Vec<TextEdit>, EditError> {
    verify_fresh(text, task)?;
    let c = column.trim().to_lowercase();
    if c == done_column.trim().to_lowercase() {
        Ok(finish(text, task, today, true, StatusChange::Remove))
    } else if c == todo_column.trim().to_lowercase() {
        Ok(finish(text, task, today, false, StatusChange::Remove))
    } else {
        Ok(finish(text, task, today, false, StatusChange::Set(c)))
    }
}

fn finish(text: &str, task: &Task, today: Date, done: bool, status: StatusChange) -> Vec<TextEdit> {
    let mut line = LineEditor::new(task);
    if done {
        line.set_checkbox('x');
        // Only a transition from open to done records a completion date.
        if !task.done && task.completed.is_none() {
            line.upsert_field("done", &today.to_string());
        }
    } else {
        line.set_checkbox(' ');
        line.remove_field("done");
    }
    match &status {
        StatusChange::Keep => {}
        StatusChange::Remove => line.remove_field("status"),
        StatusChange::Set(s) => line.upsert_field("status", s),
    }

    let mut replacement = line.raw.clone();
    // Completing a recurring task (open -> done) schedules the next occurrence.
    if done && !task.done {
        if let Some(rule) = task.repeat {
            let eol = if text[task.span.end..].starts_with("\r\n") { "\r\n" } else { "\n" };
            let old_due = task.due.map(|d| d.date());
            let base = if rule.from_completion {
                today
            } else {
                old_due.unwrap_or(today)
            };
            let next_date = rule.next(base);
            let new_due = match task.due {
                Some(d) => d.with_date(next_date),
                None => TaskDate::Date(next_date),
            };
            let mut next = LineEditor::new(task);
            next.set_checkbox(' ');
            next.remove_field("done");
            next.remove_field("status");
            next.strip_id();
            next.upsert_field("due", &new_due.to_string());
            if let Some(start) = task.start {
                let delta = next_date.to_days() - old_due.unwrap_or(base).to_days();
                next.upsert_field("start", &start.with_date(start.date().add_days(delta)).to_string());
            }
            replacement = format!("{}{}{}", replacement, eol, next.raw);
        }
    }

    if replacement == task.raw {
        return Vec::new();
    }
    vec![TextEdit {
        start: task.span.start,
        end: task.span.end,
        replacement,
    }]
}

/// Gives every task without an id a stable one (` ^t-xxxxx` at the end of its line). Ids are
/// derived deterministically from `seed` (use the note's path), the title and the line, and never
/// collide with `taken`. Stale tasks are skipped. This is only ever run on explicit request.
pub fn assign_ids(text: &str, tasks: &[Task], taken: &HashSet<String>, seed: &str) -> Vec<TextEdit> {
    let mut used: HashSet<String> = taken.clone();
    let mut edits = Vec::new();
    for t in tasks {
        if let Some(id) = &t.id {
            used.insert(id.clone());
        }
    }
    for t in tasks {
        if t.id.is_some() || verify_fresh(text, t).is_err() {
            continue;
        }
        let id = make_id(&format!("{}|{}|{}", seed, t.title, t.line), &used);
        used.insert(id.clone());
        edits.push(TextEdit {
            start: t.span.start,
            end: t.span.end,
            replacement: format!("{} ^{}", t.raw.trim_end(), id),
        });
    }
    edits
}

fn make_id(seed: &str, used: &HashSet<String>) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut n = 0u32;
    loop {
        // FNV-1a, 64 bit.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in format!("{}#{}", seed, n).bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut s = String::from("t-");
        let mut x = h;
        for _ in 0..5 {
            s.push(DIGITS[(x % 36) as usize] as char);
            x /= 36;
        }
        if !used.contains(&s) || n > 100_000 {
            return s;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::tasks::parse_tasks;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn first_task(text: &str) -> Task {
        parse_tasks(&parse(text)).tasks.remove(0)
    }

    fn apply(text: &str, edits: Vec<TextEdit>) -> String {
        apply_edits(text, &edits).unwrap()
    }

    #[test]
    fn apply_edits_rules() {
        let text = "héllo world";
        let e = |s, t, r: &str| TextEdit { start: s, end: t, replacement: r.to_string() };
        assert_eq!(apply_edits(text, &[e(0, 0, ">> ")]).unwrap(), ">> héllo world");
        assert_eq!(apply_edits(text, &[e(7, 12, "there"), e(0, 6, "Hi")]).unwrap(), "Hi there");
        assert_eq!(apply_edits(text, &[e(0, 5, "a"), e(3, 6, "b")]), Err(EditError::Overlap));
        assert_eq!(apply_edits(text, &[e(2, 3, "x")]), Err(EditError::InvalidRange)); // inside é
        assert_eq!(apply_edits(text, &[e(0, 99, "x")]), Err(EditError::InvalidRange));
    }

    #[test]
    fn complete_and_reopen() {
        let text = "- [ ] a ^t-aa\n";
        let t = first_task(text);
        let done = apply(text, set_done(text, &t, true, d("2026-11-03")).unwrap());
        assert_eq!(done, "- [x] a [done: 2026-11-03] ^t-aa\n");
        let t2 = first_task(&done);
        let reopened = apply(&done, set_done(&done, &t2, false, d("2026-11-04")).unwrap());
        assert_eq!(reopened, "- [ ] a ^t-aa\n");
    }

    #[test]
    fn completing_keeps_an_existing_completion_date_and_is_idempotent() {
        let text = "- [x] a [done: 2026-10-20]\n";
        let t = first_task(text);
        assert!(set_done(text, &t, true, d("2026-11-03")).unwrap().is_empty());
    }

    #[test]
    fn only_the_task_line_changes() {
        let text = "# Title\n\nSome *text*\n\n- [ ] one\n- [ ] two [due: 2026-11-03]\n  - [ ] sub\n\ntail\n";
        let tasks = parse_tasks(&parse(text)).tasks;
        let edits = set_done(text, &tasks[1], true, d("2026-11-01")).unwrap();
        let out = apply(text, edits);
        assert_eq!(
            out,
            "# Title\n\nSome *text*\n\n- [ ] one\n- [x] two [due: 2026-11-03] [done: 2026-11-01]\n  - [ ] sub\n\ntail\n"
        );
    }

    #[test]
    fn stale_tasks_are_rejected() {
        let text = "- [ ] a\n";
        let t = first_task(text);
        let changed = "- [ ] b\n";
        assert_eq!(set_done(changed, &t, true, d("2026-11-03")), Err(EditError::Stale));
        let shifted = "new line\n- [ ] a\n";
        assert_eq!(set_done(shifted, &t, true, d("2026-11-03")), Err(EditError::Stale));
    }

    #[test]
    fn recurring_tasks_schedule_the_next_occurrence() {
        let text = "- [ ] water [due: 2026-11-03] [start: 2026-11-01] [repeat: weekly] ^t-w1\nnext\n";
        let t = first_task(text);
        let out = apply(text, set_done(text, &t, true, d("2026-11-05")).unwrap());
        assert_eq!(
            out,
            "- [x] water [due: 2026-11-03] [start: 2026-11-01] [repeat: weekly] [done: 2026-11-05] ^t-w1\n- [ ] water [due: 2026-11-10] [start: 2026-11-08] [repeat: weekly]\nnext\n"
        );
    }

    #[test]
    fn recurrence_from_completion_keeps_time_and_crlf() {
        let text = "- [ ] standup [due: 2026-11-03T09:00+01:00] [repeat: every 2 days when done]\r\nrest\r\n";
        let t = first_task(text);
        let out = apply(text, set_done(text, &t, true, d("2026-11-05")).unwrap());
        assert_eq!(
            out,
            "- [x] standup [due: 2026-11-03T09:00+01:00] [repeat: every 2 days when done] [done: 2026-11-05]\r\n- [ ] standup [due: 2026-11-07T09:00+01:00] [repeat: every 2 days when done]\r\nrest\r\n"
        );
    }

    #[test]
    fn recurring_without_a_due_date_gets_one() {
        let text = "- [ ] stretch [repeat: daily]\n";
        let t = first_task(text);
        let out = apply(text, set_done(text, &t, true, d("2026-11-05")).unwrap());
        assert_eq!(
            out,
            "- [x] stretch [repeat: daily] [done: 2026-11-05]\n- [ ] stretch [repeat: daily] [due: 2026-11-06]\n"
        );
    }

    #[test]
    fn reopening_a_recurring_task_does_not_add_lines() {
        let text = "- [x] a [due: 2026-11-03] [repeat: weekly] [done: 2026-11-03]\n";
        let t = first_task(text);
        let out = apply(text, set_done(text, &t, false, d("2026-11-05")).unwrap());
        assert_eq!(out, "- [ ] a [due: 2026-11-03] [repeat: weekly]\n");
    }

    #[test]
    fn moving_between_columns() {
        let text = "- [ ] card ^t-c1\n";
        let t = first_task(text);
        let active = apply(text, move_to_column(text, &t, "Active", "done", "todo", d("2026-11-03")).unwrap());
        assert_eq!(active, "- [ ] card [status: active] ^t-c1\n");

        let t = first_task(&active);
        let review = apply(&active, move_to_column(&active, &t, "review", "done", "todo", d("2026-11-03")).unwrap());
        assert_eq!(review, "- [ ] card [status: review] ^t-c1\n");

        let t = first_task(&review);
        let done = apply(&review, move_to_column(&review, &t, "Done", "done", "todo", d("2026-11-04")).unwrap());
        assert_eq!(done, "- [x] card [done: 2026-11-04] ^t-c1\n");

        let t = first_task(&done);
        let back = apply(&done, move_to_column(&done, &t, "active", "done", "todo", d("2026-11-05")).unwrap());
        assert_eq!(back, "- [ ] card [status: active] ^t-c1\n");

        let t = first_task(&back);
        let todo = apply(&back, move_to_column(&back, &t, "todo", "done", "todo", d("2026-11-05")).unwrap());
        assert_eq!(todo, "- [ ] card ^t-c1\n");
        // Moving to the column it is already in changes nothing.
        let t = first_task(&todo);
        assert!(move_to_column(&todo, &t, "todo", "done", "todo", d("2026-11-05")).unwrap().is_empty());
    }

    #[test]
    fn moving_never_touches_other_content() {
        let text = "intro\n\n- [ ] a [priority: high] #x ^t-aa\n  - [ ] child\n- [ ] b\n\n[status: not a task field]\n";
        let tasks = parse_tasks(&parse(text)).tasks;
        let out = apply(text, move_to_column(text, &tasks[0], "active", "done", "todo", d("2026-11-03")).unwrap());
        assert_eq!(
            out,
            "intro\n\n- [ ] a [priority: high] #x [status: active] ^t-aa\n  - [ ] child\n- [ ] b\n\n[status: not a task field]\n"
        );
        // Parsing the result still yields the same tasks, one status changed.
        let after = parse_tasks(&parse(&out)).tasks;
        assert_eq!(after.len(), tasks.len());
        assert_eq!(after[0].status.as_deref(), Some("active"));
        assert_eq!(after[0].title, tasks[0].title);
        assert_eq!(after[0].id, tasks[0].id);
    }

    #[test]
    fn assigning_ids() {
        let text = "- [ ] same\n- [ ] same\n- [ ] has ^t-keep\n";
        let tasks = parse_tasks(&parse(text)).tasks;
        let edits = assign_ids(text, &tasks, &HashSet::new(), "notes/a.md");
        assert_eq!(edits.len(), 2);
        let out = apply(text, edits.clone());
        let after = parse_tasks(&parse(&out)).tasks;
        let ids: Vec<String> = after.iter().map(|t| t.id.clone().unwrap()).collect();
        assert_eq!(ids.len(), 3);
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids[2], "t-keep");
        assert!(ids.iter().all(|i| crate::structure::is_block_id(i)));
        // Deterministic.
        assert_eq!(edits, assign_ids(text, &tasks, &HashSet::new(), "notes/a.md"));
        // Avoids taken ids.
        let mut taken = HashSet::new();
        taken.insert(ids[0].clone());
        taken.insert(ids[1].clone());
        let other = assign_ids(text, &tasks, &taken, "notes/a.md");
        let out2 = apply(text, other);
        let ids2: Vec<String> = parse_tasks(&parse(&out2)).tasks.iter().map(|t| t.id.clone().unwrap()).collect();
        assert!(!ids2[..2].contains(&ids[0]) && !ids2[..2].contains(&ids[1]));
    }

    #[test]
    fn unicode_lines_are_safe() {
        let text = "- [ ] café ☕ [due: 2026-11-03] ^t-cf\n";
        let t = first_task(text);
        let out = apply(text, set_done(text, &t, true, d("2026-11-03")).unwrap());
        assert_eq!(out, "- [x] café ☕ [due: 2026-11-03] [done: 2026-11-03] ^t-cf\n");
    }
}
