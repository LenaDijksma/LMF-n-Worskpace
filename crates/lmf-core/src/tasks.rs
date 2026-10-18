//! Tasks: Markdown checkbox list items with a small, explicit set of optional fields.
//!
//! ```markdown
//! - [ ] Write the parser [due: 2026-11-03] [priority: high] #dev ^t-parser
//!   - [x] Define the grammar [done: 2026-10-20]
//!   - [ ] Add tests [estimate: 1h30m] [after: t-parser]
//! ```
//!
//! * The line is an ordinary GitHub-style task item, so every Markdown renderer shows a normal
//!   checkbox list. Fields are plain `[key: value]` text that degrades gracefully.
//! * **Identity.** A task's stable id is its trailing block marker (`^t-parser`, see
//!   [`crate::structure`]). Tasks without one are *provisional*: found by note and position
//!   only. Ids are never invented silently; see [`crate::taskedit::assign_ids`].
//! * **Fields** (all optional): `due`, `start`, `done` (completion date) as a date or a
//!   timestamp with explicit offset ([`crate::dates`]); `priority` (low, normal, high, urgent);
//!   `status` (a free column name such as `active`); `estimate` (`90m`, `1h30m`, `1.5h`);
//!   `repeat` (`daily`, `every 2 weeks`, `monthly when done`); `after` (ids of tasks this one
//!   depends on). `#tags` in the title are collected. Unknown `[key: value]` text stays in the
//!   title untouched.
//! * **Subtasks** are checkbox items nested (by indentation) under another task.
//! * Tasks inside fenced code and unknown extension blocks are ignored; tasks inside known
//!   extension blocks (callouts, tabs, …) are found.
//!
//! Content from `lmf:include` is never indexed: a task lives only in the note whose source
//! contains its line, so transclusion cannot duplicate it.

use crate::dates::{Date, TaskDate};
use crate::diagnostics::{codes, Diagnostic};
use crate::lines::{fence_close, fence_open, indent_of, is_blank, split_lines};
use crate::model::{Block, Document};
use crate::registry;
use crate::span::{LineIndex, Span};
use crate::structure::{is_block_id, trailing_marker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Low,
    Normal,
    High,
    Urgent,
}

impl Priority {
    pub fn parse(s: &str) -> Option<Priority> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Priority::Low),
            "normal" | "medium" | "med" => Some(Priority::Normal),
            "high" => Some(Priority::High),
            "urgent" | "critical" => Some(Priority::Urgent),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatUnit {
    Day,
    Week,
    Month,
    Year,
}

/// A simple recurrence rule: every N days/weeks/months/years.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repeat {
    pub every: u32,
    pub unit: RepeatUnit,
    /// Count from the completion date instead of from the previous due date.
    pub from_completion: bool,
}

impl Repeat {
    /// `daily`, `weekly`, `monthly`, `yearly`, `every day`, `every 2 weeks`, each optionally
    /// followed by `when done`.
    pub fn parse(s: &str) -> Option<Repeat> {
        let lower = s.trim().to_ascii_lowercase();
        let (rule, from_completion) = match lower.strip_suffix("when done") {
            Some(r) => (r.trim().to_string(), true),
            None => (lower, false),
        };
        let (every, unit) = match rule.as_str() {
            "daily" => (1, RepeatUnit::Day),
            "weekly" => (1, RepeatUnit::Week),
            "monthly" => (1, RepeatUnit::Month),
            "yearly" | "annually" => (1, RepeatUnit::Year),
            other => {
                let words: Vec<&str> = other.split_whitespace().collect();
                if words.first() != Some(&"every") {
                    return None;
                }
                let (n, unit_word) = match words.len() {
                    2 => (1u32, words[1]),
                    3 => (words[1].parse::<u32>().ok()?, words[2]),
                    _ => return None,
                };
                if !(1..=365).contains(&n) {
                    return None;
                }
                let unit = match unit_word.trim_end_matches('s') {
                    "day" => RepeatUnit::Day,
                    "week" => RepeatUnit::Week,
                    "month" => RepeatUnit::Month,
                    "year" => RepeatUnit::Year,
                    _ => return None,
                };
                (n, unit)
            }
        };
        Some(Repeat {
            every,
            unit,
            from_completion,
        })
    }

    /// The next occurrence after `from`.
    pub fn next(&self, from: Date) -> Date {
        let n = self.every as i64;
        match self.unit {
            RepeatUnit::Day => from.add_days(n),
            RepeatUnit::Week => from.add_days(7 * n),
            RepeatUnit::Month => from.add_months(self.every as i32),
            RepeatUnit::Year => from.add_years(self.every as i32),
        }
    }

    pub fn describe(&self) -> String {
        let unit = match self.unit {
            RepeatUnit::Day => "day",
            RepeatUnit::Week => "week",
            RepeatUnit::Month => "month",
            RepeatUnit::Year => "year",
        };
        let base = if self.every == 1 {
            format!("every {}", unit)
        } else {
            format!("every {} {}s", self.every, unit)
        };
        if self.from_completion {
            format!("{} when done", base)
        } else {
            base
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// Stable id: the trailing `^id` marker of the line.
    pub id: Option<String>,
    /// Text without fields and without the id marker (tags are kept).
    pub title: String,
    pub done: bool,
    /// Free column name from `[status: …]`, lower-case.
    pub status: Option<String>,
    pub priority: Option<Priority>,
    pub due: Option<TaskDate>,
    pub start: Option<TaskDate>,
    /// Completion date from `[done: …]`.
    pub completed: Option<TaskDate>,
    pub estimate_minutes: Option<u32>,
    pub repeat: Option<Repeat>,
    /// Ids of tasks that must be done first (`[after: a, b]`).
    pub after: Vec<String>,
    pub tags: Vec<String>,
    /// Index of the parent task in the same [`TaskParse::tasks`] list.
    pub parent: Option<usize>,
    /// Indentation of the list item (tab = 4).
    pub indent: usize,
    /// 1-based line number.
    pub line: usize,
    /// The whole list-item line, without the line break.
    pub span: Span,
    /// The character inside `[ ]`.
    pub checkbox: Span,
    /// Offset of the text after the checkbox, relative to the start of the line.
    pub rest_offset: usize,
    /// The exact source line (used to detect a stale task before editing).
    pub raw: String,
}

impl Task {
    /// `done`, or the `status` field, or `todo`.
    pub fn status_name(&self) -> &str {
        if self.done {
            "done"
        } else {
            self.status.as_deref().unwrap_or("todo")
        }
    }

    pub fn is_open(&self) -> bool {
        !self.done
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskParse {
    pub tasks: Vec<Task>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Finds all tasks of a document, in source order, with diagnostics for invalid fields.
pub fn parse_tasks(doc: &Document) -> TaskParse {
    let mut out = TaskParse::default();
    collect(doc, &doc.blocks, &mut out);
    let index = LineIndex::new(&doc.source);
    for t in &mut out.tasks {
        t.line = index.line_col(&doc.source, t.span.start).0;
    }
    out
}

/// Progress of a task's direct subtasks: `(done, total)`, or `None` without subtasks.
pub fn progress(tasks: &[Task], index: usize) -> Option<(usize, usize)> {
    let children: Vec<&Task> = tasks.iter().filter(|t| t.parent == Some(index)).collect();
    if children.is_empty() {
        None
    } else {
        Some((children.iter().filter(|t| t.done).count(), children.len()))
    }
}

fn collect(doc: &Document, blocks: &[Block], out: &mut TaskParse) {
    for block in blocks {
        match block {
            Block::Markdown(m) => scan_block(doc.text(m.span), m.span.start, out),
            Block::Directive(d) => {
                if registry::lookup(&d.namespace, &d.name).is_some() {
                    collect(doc, &d.body, out);
                }
            }
        }
    }
}

struct ListItem {
    indent: usize,
    /// Byte offset of the character inside `[ ]` and that character.
    checkbox: Option<(usize, char)>,
    rest_start: usize,
}

fn parse_list_item(line: &str) -> Option<ListItem> {
    let b = line.as_bytes();
    let mut indent = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b' ' => {
                indent += 1;
                i += 1;
            }
            b'\t' => {
                indent += 4;
                i += 1;
            }
            _ => break,
        }
    }
    let first = *b.get(i)?;
    let marker_end = match first {
        b'-' | b'*' | b'+' => i + 1,
        b'0'..=b'9' => {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j - i > 9 || !matches!(b.get(j), Some(&b'.') | Some(&b')')) {
                return None;
            }
            j + 1
        }
        _ => return None,
    };
    if !matches!(b.get(marker_end), Some(&b' ') | Some(&b'\t')) {
        return None;
    }
    let mut j = marker_end;
    while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    if b.len() >= j + 3
        && b[j] == b'['
        && b[j + 2] == b']'
        && matches!(b[j + 1], b' ' | b'x' | b'X')
        && (b.len() == j + 3 || b[j + 3] == b' ' || b[j + 3] == b'\t')
    {
        let mut k = j + 3;
        while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
            k += 1;
        }
        return Some(ListItem {
            indent,
            checkbox: Some((j + 1, b[j + 1] as char)),
            rest_start: k,
        });
    }
    Some(ListItem {
        indent,
        checkbox: None,
        rest_start: j,
    })
}

fn scan_block(text: &str, base: usize, out: &mut TaskParse) {
    let mut fence: Option<(char, usize)> = None;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for line in split_lines(text) {
        if let Some((ch, len)) = fence {
            if fence_close(line.text, ch, len) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = fence_open(line.text) {
            fence = Some(open);
            continue;
        }
        if is_blank(line.text) {
            continue;
        }
        match parse_list_item(line.text) {
            Some(item) => {
                while let Some(&(ind, _)) = stack.last() {
                    if ind >= item.indent {
                        stack.pop();
                    } else {
                        break;
                    }
                }
                if let Some((cb_off, cb_char)) = item.checkbox {
                    let parent = stack.last().map(|&(_, i)| i);
                    let idx = out.tasks.len();
                    let task = build_task(line.text, base + line.start, &item, cb_off, cb_char, parent, out);
                    out.tasks.push(task);
                    stack.push((item.indent, idx));
                }
            }
            None => {
                if indent_of(line.text) == 0 {
                    stack.clear();
                }
            }
        }
    }
}

fn build_task(
    line: &str,
    abs_start: usize,
    item: &ListItem,
    cb_off: usize,
    cb_char: char,
    parent: Option<usize>,
    out: &mut TaskParse,
) -> Task {
    let rest = &line[item.rest_start..];
    let (body, id) = match trailing_marker(rest) {
        Some((caret, id)) => (rest[..caret].trim_end(), Some(id.to_string())),
        None => (rest.trim_end(), None),
    };

    let mut task = Task {
        id,
        title: String::new(),
        done: cb_char != ' ',
        status: None,
        priority: None,
        due: None,
        start: None,
        completed: None,
        estimate_minutes: None,
        repeat: None,
        after: Vec::new(),
        tags: Vec::new(),
        parent,
        indent: item.indent,
        line: 0,
        span: Span::new(abs_start, abs_start + line.len()),
        checkbox: Span::new(abs_start + cb_off, abs_start + cb_off + 1),
        rest_offset: item.rest_start,
        raw: line.to_string(),
    };

    let seg_span = |seg: &FieldSeg| {
        Span::new(
            abs_start + item.rest_start + seg.start,
            abs_start + item.rest_start + seg.end,
        )
    };

    let mut title_raw = String::new();
    let mut last = 0usize;
    let mut seen: Vec<String> = Vec::new();
    for seg in find_fields(body) {
        title_raw.push_str(&body[last..seg.start]);
        last = seg.end;
        let span = seg_span(&seg);
        if seen.contains(&seg.key) {
            out.diagnostics.push(Diagnostic::warning(
                codes::INVALID_TASK_FIELD,
                format!("`{}` is given more than once; the first value is used", seg.key),
                span,
            ));
            continue;
        }
        seen.push(seg.key.clone());
        let bad = |what: &str| {
            Diagnostic::warning(
                codes::INVALID_TASK_FIELD,
                format!("invalid `{}` value `{}`: {}", seg.key, seg.value, what),
                span,
            )
        };
        match seg.key.as_str() {
            "due" | "start" | "done" => match TaskDate::parse(&seg.value) {
                Ok(d) => match seg.key.as_str() {
                    "due" => task.due = Some(d),
                    "start" => task.start = Some(d),
                    _ => task.completed = Some(d),
                },
                Err(e) => out.diagnostics.push(bad(&e.to_string())),
            },
            "priority" => match Priority::parse(&seg.value) {
                Some(p) => task.priority = Some(p),
                None => out
                    .diagnostics
                    .push(bad("expected low, normal, high or urgent")),
            },
            "status" => {
                let v = seg.value.trim().to_lowercase();
                let valid = !v.is_empty()
                    && v.len() <= 32
                    && v.chars()
                        .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_');
                if valid {
                    task.status = Some(v);
                } else {
                    out.diagnostics
                        .push(bad("use a short name of letters, digits, spaces, - or _"));
                }
            }
            "estimate" => match parse_estimate(&seg.value) {
                Some(m) => task.estimate_minutes = Some(m),
                None => out
                    .diagnostics
                    .push(bad("expected something like 90m, 2h or 1h30m")),
            },
            "repeat" => match Repeat::parse(&seg.value) {
                Some(r) => task.repeat = Some(r),
                None => out.diagnostics.push(bad(
                    "expected daily, weekly, monthly, yearly or `every N days|weeks|months|years`",
                )),
            },
            "after" => {
                let ids: Vec<&str> = seg
                    .value
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .collect();
                if ids.is_empty() || !ids.iter().all(|i| is_block_id(i)) {
                    out.diagnostics
                        .push(bad("expected one or more task ids, e.g. t-abc12"));
                } else {
                    task.after = ids.iter().map(|s| s.to_string()).collect();
                }
            }
            _ => {}
        }
    }
    title_raw.push_str(&body[last..]);
    task.title = title_raw.split_whitespace().collect::<Vec<&str>>().join(" ");
    task.tags = collect_tags(&task.title);
    task
}

fn collect_tags(title: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for word in title.split_whitespace() {
        if let Some(t) = word.strip_prefix('#') {
            let t = t.trim_end_matches(|c: char| c == '.' || c == ',' || c == ';' || c == ':' || c == '!' || c == '?');
            if !t.is_empty()
                && t.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '/')
            {
                let t = t.to_lowercase();
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
    }
    tags
}

/// `90m`, `2h`, `1h30m`, `1.5h` → minutes.
pub fn parse_estimate(s: &str) -> Option<u32> {
    let t: String = s.trim().to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return None;
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut seen = false;
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else if c == 'h' || c == 'm' {
            let v: f64 = num.parse().ok()?;
            num.clear();
            total += if c == 'h' { v * 60.0 } else { v };
            seen = true;
        } else {
            return None;
        }
    }
    if !num.is_empty() || !seen {
        return None;
    }
    let minutes = total.round();
    if minutes <= 0.0 || minutes > 100_000.0 {
        return None;
    }
    Some(minutes as u32)
}

/// `90` → `1h30m`, `120` → `2h`, `45` → `45m`.
pub fn format_estimate(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{}m", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h{}m", h, m),
    }
}

/// A `[key: value]` field inside a task line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldSeg {
    pub key: String,
    pub value: String,
    /// Offsets of the whole `[key: value]` within the scanned string.
    pub start: usize,
    pub end: usize,
}

pub(crate) const FIELD_KEYS: &[&str] = &[
    "due", "start", "done", "priority", "status", "estimate", "repeat", "after",
];

/// Finds known `[key: value]` fields (not inside code spans, not link text).
pub(crate) fn find_fields(s: &str) -> Vec<FieldSeg> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'`' {
            let run = b[i..].iter().take_while(|&&x| x == b'`').count();
            let mut j = i + run;
            let mut closed = None;
            while j < b.len() {
                if b[j] == b'`' {
                    let n = b[j..].iter().take_while(|&&x| x == b'`').count();
                    if n == run {
                        closed = Some(j + n);
                        break;
                    }
                    j += n;
                } else {
                    j += 1;
                }
            }
            i = closed.unwrap_or(i + run);
            continue;
        }
        if b[i] == b'[' {
            if let Some(rel) = s[i + 1..].find(|c: char| c == ']' || c == '[') {
                let close = i + 1 + rel;
                if b[close] == b']' {
                    let inner = &s[i + 1..close];
                    let end = close + 1;
                    if b.get(end) != Some(&b'(') {
                        if let Some((k, v)) = inner.split_once(':') {
                            let key = k.trim().to_ascii_lowercase();
                            if FIELD_KEYS.contains(&key.as_str()) {
                                out.push(FieldSeg {
                                    key,
                                    value: v.trim().to_string(),
                                    start: i,
                                    end,
                                });
                                i = end;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn tasks(src: &str) -> TaskParse {
        parse_tasks(&parse(src))
    }

    const SAMPLE: &str = "# Plan\n\n- [ ] Write parser [due: 2026-11-03] [priority: high] #dev ^t-abc12\n  - [x] Define grammar [done: 2026-10-20]\n  - [ ] Add tests [estimate: 1h30m] [after: t-abc12]\n- [x] Choose name\nPlain paragraph\n- [ ] Bad date [due: tomorrow]\n";

    #[test]
    fn parses_fields_ids_tags_and_structure() {
        let p = tasks(SAMPLE);
        assert_eq!(p.tasks.len(), 5);
        let t0 = &p.tasks[0];
        assert_eq!(t0.id.as_deref(), Some("t-abc12"));
        assert_eq!(t0.title, "Write parser #dev");
        assert_eq!(t0.tags, vec!["dev"]);
        assert_eq!(t0.priority, Some(Priority::High));
        assert_eq!(t0.due, TaskDate::parse("2026-11-03").ok());
        assert!(!t0.done);
        assert_eq!(t0.line, 3);
        assert_eq!(t0.parent, None);

        let t1 = &p.tasks[1];
        assert!(t1.done);
        assert_eq!(t1.parent, Some(0));
        assert_eq!(t1.completed, TaskDate::parse("2026-10-20").ok());

        let t2 = &p.tasks[2];
        assert_eq!(t2.estimate_minutes, Some(90));
        assert_eq!(t2.after, vec!["t-abc12"]);
        assert_eq!(t2.parent, Some(0));

        assert_eq!(p.tasks[3].title, "Choose name");
        assert_eq!(p.tasks[3].parent, None);
        assert_eq!(p.tasks[4].parent, None);
        assert_eq!(p.tasks[4].due, None);
        assert_eq!(p.diagnostics.len(), 1);
        assert_eq!(p.diagnostics[0].code, "LMF021");
    }

    #[test]
    fn spans_and_raw_match_the_source() {
        let doc = parse(SAMPLE);
        let p = parse_tasks(&doc);
        for t in &p.tasks {
            assert_eq!(doc.text(t.span), t.raw);
            let cb = doc.text(t.checkbox);
            assert!(cb == " " || cb == "x", "{:?}", cb);
        }
        assert_eq!(progress(&p.tasks, 0), Some((1, 2)));
        assert_eq!(progress(&p.tasks, 3), None);
    }

    #[test]
    fn status_and_default_status() {
        let p = tasks("- [ ] a [status: In Progress]\n- [ ] b\n- [x] c [status: active]\n");
        assert_eq!(p.tasks[0].status_name(), "in progress");
        assert_eq!(p.tasks[1].status_name(), "todo");
        assert_eq!(p.tasks[2].status_name(), "done");
    }

    #[test]
    fn code_and_unknown_blocks_are_ignored_known_blocks_count() {
        let p = tasks("```\n- [ ] in code\n```\n\n:::vendor:x\n- [ ] hidden\n:::\n\n:::lmf:callout\n- [ ] in callout\n:::\n\n`- [ ] inline`\n");
        assert_eq!(p.tasks.len(), 1);
        assert_eq!(p.tasks[0].title, "in callout");
    }

    #[test]
    fn unknown_brackets_and_links_stay_in_the_title() {
        let p = tasks("- [ ] See [the doc](https://x.org) and [ref: 12] and [[Note]] [due: 2026-01-01]\n");
        assert_eq!(p.tasks[0].title, "See [the doc](https://x.org) and [ref: 12] and [[Note]]");
        assert_eq!(p.tasks[0].due, TaskDate::parse("2026-01-01").ok());
        assert!(p.diagnostics.is_empty());
    }

    #[test]
    fn invalid_and_duplicate_fields_are_reported() {
        let p = tasks("- [ ] a [due: 2026-02-30] [priority: soon] [estimate: 5] [repeat: whenever] [after: x y!] [status: ] [due: 2026-01-01] [due: 2026-01-02] [due: 2026-03-03T10:00]\n");
        let t = &p.tasks[0];
        assert_eq!(t.due, None);
        assert_eq!(t.priority, None);
        // 6 invalid values + the first valid `due` given after an invalid one is repeated -> see below
        assert!(p.diagnostics.iter().all(|d| d.code == "LMF021"));
        assert!(p.diagnostics.len() >= 7, "{:?}", p.diagnostics);
    }

    #[test]
    fn timestamps_need_offsets() {
        let ok = tasks("- [ ] a [due: 2026-11-03T14:00+01:00]\n");
        assert!(ok.diagnostics.is_empty());
        assert!(!ok.tasks[0].due.unwrap().is_date_only());
        let bad = tasks("- [ ] a [due: 2026-11-03T14:00]\n");
        assert_eq!(bad.diagnostics.len(), 1);
        assert!(bad.diagnostics[0].message.contains("offset"));
    }

    #[test]
    fn list_variants() {
        let p = tasks("* [ ] star\n+ [X] plus\n1. [ ] numbered\n2) [x] paren\n- [ ]no space\n- [] empty box\n-  [ ]  two spaces\n");
        let titles: Vec<&str> = p.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["star", "plus", "numbered", "paren", "two spaces"]);
        assert!(p.tasks[1].done && p.tasks[3].done);
    }

    #[test]
    fn nesting_resets_at_plain_text_and_sibling_lists() {
        let p = tasks("- [ ] a\n    - [ ] deep\n  - [ ] mid\n- [ ] b\n  - [ ] child of b\n\ntext\n  - [ ] orphan\n");
        let parents: Vec<Option<usize>> = p.tasks.iter().map(|t| t.parent).collect();
        assert_eq!(parents, vec![None, Some(0), Some(0), None, Some(3), None]);
    }

    #[test]
    fn repeat_rules() {
        let d = |s: &str| Date::parse(s).unwrap();
        assert_eq!(Repeat::parse("daily").unwrap().next(d("2026-11-30")), d("2026-12-01"));
        assert_eq!(Repeat::parse("every 2 weeks").unwrap().next(d("2026-11-03")), d("2026-11-17"));
        assert_eq!(Repeat::parse("monthly").unwrap().next(d("2026-01-31")), d("2026-02-28"));
        assert_eq!(Repeat::parse("every year").unwrap().next(d("2028-02-29")), d("2029-02-28"));
        let r = Repeat::parse("every 3 months when done").unwrap();
        assert!(r.from_completion);
        assert_eq!(r.describe(), "every 3 months when done");
        for bad in ["", "sometimes", "every", "every 0 days", "every x days", "every 2 fortnights", "every 2 3 days"] {
            assert_eq!(Repeat::parse(bad), None, "{}", bad);
        }
    }

    #[test]
    fn estimates() {
        assert_eq!(parse_estimate("90m"), Some(90));
        assert_eq!(parse_estimate("2h"), Some(120));
        assert_eq!(parse_estimate("1h30m"), Some(90));
        assert_eq!(parse_estimate("1.5h"), Some(90));
        assert_eq!(parse_estimate("1h 30m"), Some(90));
        for bad in ["", "30", "h", "0m", "abc", "1h2", "-5m"] {
            assert_eq!(parse_estimate(bad), None, "{}", bad);
        }
        assert_eq!(format_estimate(90), "1h30m");
        assert_eq!(format_estimate(120), "2h");
        assert_eq!(format_estimate(45), "45m");
    }

    #[test]
    fn never_panics() {
        for t in ["- [", "- [ ]", "- [ ] [", "- [ ] [due:", "- [ ] ^", "- [ ] é [due: é] ^é", "1. [x]", "- [ ] `[due: 2026-01-01]`", "\t- [ ] tab"] {
            let _ = tasks(t);
        }
    }
}
