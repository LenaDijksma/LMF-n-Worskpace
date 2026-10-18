//! The task query language: filters, search terms and sorting in one line of text.
//!
//! ```text
//! status:active tag:work due:week sort:due        tasks in the "active" column tagged #work, due this week
//! is:open -tag:someday "release notes"            open tasks without #someday mentioning the phrase
//! due:overdue priority:high sort:-priority        overdue high-priority tasks
//! due:2026-11-01..2026-11-30 note:projects/       tasks due in November from notes under projects/
//! ```
//!
//! Terms are separated by spaces; a term is a filter (`key:value`) or free text (all free-text
//! terms must occur in the title, case-insensitively). Quote text with spaces: `"two words"` or
//! `note:"my folder"`. A leading `-` negates a term.
//!
//! | filter | meaning |
//! |--------|---------|
//! | `status:NAME` | column/status (`todo` when unset, `done` when checked, `open` = not done) |
//! | `is:` | `done` `open` `blocked` `overdue` `recurring` `root` `sub` |
//! | `tag:NAME` | has the tag (with or without `#`) |
//! | `priority:P` | `low` `normal` `high` `urgent` |
//! | `due:` `start:` `done:` | date filter, see below |
//! | `note:TEXT` | the note's path contains TEXT |
//! | `id:ID` | the task's id |
//! | `has:` | `due` `start` `done` `tag` `priority` `status` `estimate` `repeat` `id` |
//! | `sort:` | `due` `start` `priority` `title` `note` `status`; `-` reverses |
//!
//! Date filters (days, never time zones — date-only values are floating): `today`, `tomorrow`,
//! `yesterday`, `overdue` (before today and still open), `week` (this week, Monday–Sunday),
//! `next7` (today through today+7), `none`, `2026-11-03`, `<2026-11-03`, `>2026-11-03`,
//! `2026-11-01..2026-11-30` (inclusive). `<`, `>` and `..` also accept `today` etc.
//! A timestamp is compared by its calendar day in its own stated offset.

use std::cmp::Ordering;

use crate::dates::Date;
use crate::tasks::{Priority, Task};

/// What "now" means for a query. Passed in so queries are deterministic and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctx {
    pub today: Date,
}

/// A task together with the context a query needs.
#[derive(Debug, Clone, Copy)]
pub struct TaskView<'a> {
    pub task: &'a Task,
    pub note_path: &'a str,
    /// Some task listed in `after` is not done yet.
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub message: String,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, QueryError> {
    Err(QueryError {
        message: message.into(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DayRef {
    Abs(Date),
    Today,
    Tomorrow,
    Yesterday,
}

impl DayRef {
    fn parse(s: &str) -> Option<DayRef> {
        match s {
            "today" => Some(DayRef::Today),
            "tomorrow" => Some(DayRef::Tomorrow),
            "yesterday" => Some(DayRef::Yesterday),
            _ => Date::parse(s).map(DayRef::Abs),
        }
    }

    fn resolve(self, today: Date) -> Date {
        match self {
            DayRef::Abs(d) => d,
            DayRef::Today => today,
            DayRef::Tomorrow => today.add_days(1),
            DayRef::Yesterday => today.add_days(-1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateSpec {
    None,
    Overdue,
    Week,
    Next7,
    On(DayRef),
    Before(DayRef),
    After(DayRef),
    Range(DayRef, DayRef),
}

impl DateSpec {
    fn parse(s: &str) -> Result<DateSpec, QueryError> {
        let bad = || {
            err::<DateSpec>(format!(
                "`{}` is not a date filter; use today, tomorrow, yesterday, overdue, week, next7, none, YYYY-MM-DD, <DATE, >DATE or DATE..DATE",
                s
            ))
        };
        match s {
            "none" => return Ok(DateSpec::None),
            "overdue" => return Ok(DateSpec::Overdue),
            "week" => return Ok(DateSpec::Week),
            "next7" => return Ok(DateSpec::Next7),
            _ => {}
        }
        if let Some((a, b)) = s.split_once("..") {
            return match (DayRef::parse(a), DayRef::parse(b)) {
                (Some(x), Some(y)) => Ok(DateSpec::Range(x, y)),
                _ => bad(),
            };
        }
        if let Some(r) = s.strip_prefix('<') {
            return DayRef::parse(r).map(DateSpec::Before).map_or_else(bad, Ok);
        }
        if let Some(r) = s.strip_prefix('>') {
            return DayRef::parse(r).map(DateSpec::After).map_or_else(bad, Ok);
        }
        DayRef::parse(s).map(DateSpec::On).map_or_else(bad, Ok)
    }

    fn matches(self, date: Option<Date>, today: Date, open: bool) -> bool {
        if self == DateSpec::None {
            return date.is_none();
        }
        let Some(d) = date else {
            return false;
        };
        match self {
            DateSpec::None => false,
            DateSpec::Overdue => d < today && open,
            DateSpec::Week => {
                let start = today.week_start();
                d >= start && d <= start.add_days(6)
            }
            DateSpec::Next7 => d >= today && d <= today.add_days(7),
            DateSpec::On(r) => d == r.resolve(today),
            DateSpec::Before(r) => d < r.resolve(today),
            DateSpec::After(r) => d > r.resolve(today),
            DateSpec::Range(a, b) => {
                let (a, b) = (a.resolve(today), b.resolve(today));
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                d >= lo && d <= hi
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsKind {
    Done,
    Open,
    Blocked,
    Overdue,
    Recurring,
    Root,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HasKind {
    Due,
    Start,
    Done,
    Tag,
    Priority,
    Status,
    Estimate,
    Repeat,
    Id,
}

#[derive(Debug, Clone, PartialEq)]
enum Filter {
    Text(String),
    Status(String),
    Is(IsKind),
    Tag(String),
    Priority(Priority),
    Due(DateSpec),
    Start(DateSpec),
    Completed(DateSpec),
    Note(String),
    Id(String),
    Has(HasKind),
}

#[derive(Debug, Clone, PartialEq)]
struct Clause {
    negate: bool,
    filter: Filter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortField {
    Due,
    Start,
    Priority,
    Title,
    Note,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SortKey {
    field: SortField,
    descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskQuery {
    clauses: Vec<Clause>,
    sorts: Vec<SortKey>,
    source: String,
}

const VALID_KEYS: &str = "status, is, tag, priority, due, start, done, note, id, has, sort";

impl TaskQuery {
    /// Matches every task.
    pub fn all() -> TaskQuery {
        TaskQuery {
            clauses: Vec::new(),
            sorts: Vec::new(),
            source: String::new(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn parse(input: &str) -> Result<TaskQuery, QueryError> {
        let mut q = TaskQuery {
            clauses: Vec::new(),
            sorts: Vec::new(),
            source: input.trim().to_string(),
        };
        for raw in tokenize(input) {
            let (negate, token) = match raw.strip_prefix('-') {
                Some(rest) if !rest.is_empty() && !raw.starts_with("--") => (true, rest.to_string()),
                _ => (false, raw.clone()),
            };
            // A quoted token is free text. `key:value` needs an alphabetic key.
            let quoted = token.starts_with('\u{1}');
            let token = token.trim_start_matches('\u{1}').to_string();
            if quoted {
                q.clauses.push(Clause {
                    negate,
                    filter: Filter::Text(token.to_lowercase()),
                });
                continue;
            }
            match token.split_once(':') {
                Some((key, value))
                    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic()) =>
                {
                    q.add_filter(negate, &key.to_ascii_lowercase(), value)?;
                }
                _ => q.clauses.push(Clause {
                    negate,
                    filter: Filter::Text(token.to_lowercase()),
                }),
            }
        }
        Ok(q)
    }

    fn add_filter(&mut self, negate: bool, key: &str, value: &str) -> Result<(), QueryError> {
        let v = value.trim();
        let lower = v.to_lowercase();
        if v.is_empty() {
            return err(format!("`{}:` needs a value", key));
        }
        let filter = match key {
            "status" => Filter::Status(lower),
            "is" => Filter::Is(match lower.as_str() {
                "done" => IsKind::Done,
                "open" => IsKind::Open,
                "blocked" => IsKind::Blocked,
                "overdue" => IsKind::Overdue,
                "recurring" => IsKind::Recurring,
                "root" => IsKind::Root,
                "sub" | "subtask" => IsKind::Sub,
                _ => {
                    return err(format!(
                        "`is:{}` is not known; use done, open, blocked, overdue, recurring, root or sub",
                        v
                    ))
                }
            }),
            "tag" => Filter::Tag(lower.trim_start_matches('#').to_string()),
            "priority" => match Priority::parse(&lower) {
                Some(p) => Filter::Priority(p),
                None => return err(format!("`priority:{}` is not known; use low, normal, high or urgent", v)),
            },
            "due" => Filter::Due(DateSpec::parse(&lower)?),
            "start" => Filter::Start(DateSpec::parse(&lower)?),
            "done" => Filter::Completed(DateSpec::parse(&lower)?),
            "note" => Filter::Note(lower),
            "id" => Filter::Id(v.to_string()),
            "has" => Filter::Has(match lower.as_str() {
                "due" => HasKind::Due,
                "start" => HasKind::Start,
                "done" => HasKind::Done,
                "tag" | "tags" => HasKind::Tag,
                "priority" => HasKind::Priority,
                "status" => HasKind::Status,
                "estimate" => HasKind::Estimate,
                "repeat" => HasKind::Repeat,
                "id" => HasKind::Id,
                _ => {
                    return err(format!(
                        "`has:{}` is not known; use due, start, done, tag, priority, status, estimate, repeat or id",
                        v
                    ))
                }
            }),
            "sort" => {
                let (descending, name) = match lower.strip_prefix('-') {
                    Some(n) => (true, n),
                    None => (false, lower.as_str()),
                };
                let field = match name {
                    "due" => SortField::Due,
                    "start" => SortField::Start,
                    "priority" => SortField::Priority,
                    "title" => SortField::Title,
                    "note" => SortField::Note,
                    "status" => SortField::Status,
                    _ => {
                        return err(format!(
                            "`sort:{}` is not known; use due, start, priority, title, note or status (prefix - to reverse)",
                            v
                        ))
                    }
                };
                self.sorts.push(SortKey { field, descending });
                return Ok(());
            }
            other => {
                return err(format!(
                    "unknown filter `{}:`; valid filters are {} (quote text that contains a colon)",
                    other, VALID_KEYS
                ))
            }
        };
        self.clauses.push(Clause { negate, filter });
        Ok(())
    }

    pub fn matches(&self, view: &TaskView<'_>, ctx: &Ctx) -> bool {
        self.clauses
            .iter()
            .all(|c| matches_filter(&c.filter, view, ctx) != c.negate)
    }

    /// Sorts by the query's `sort:` keys, then by the default order (open first, due date,
    /// priority, note, line) so the result is always deterministic.
    pub fn sort(&self, items: &mut [TaskView<'_>]) {
        items.sort_by(|a, b| self.compare(a, b));
    }

    /// The ordering used by [`TaskQuery::sort`], for callers that sort richer values.
    pub fn compare(&self, a: &TaskView<'_>, b: &TaskView<'_>) -> Ordering {
        for key in &self.sorts {
            let ord = compare_by(key.field, a, b);
            let ord = if key.descending { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        default_order(a, b)
    }
}

/// Splits on spaces; `"quoted text"` becomes one token marked with a leading `\u{1}`; a quote
/// inside `key:"a b"` keeps its key.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut token_started_with_quote = false;
    for c in input.chars() {
        if c == '"' {
            if !in_quote && cur.is_empty() {
                token_started_with_quote = true;
            }
            in_quote = !in_quote;
        } else if c.is_whitespace() && !in_quote {
            if !cur.is_empty() || token_started_with_quote {
                tokens.push(if token_started_with_quote {
                    format!("\u{1}{}", cur)
                } else {
                    cur.clone()
                });
            }
            cur.clear();
            token_started_with_quote = false;
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() || token_started_with_quote {
        tokens.push(if token_started_with_quote {
            format!("\u{1}{}", cur)
        } else {
            cur
        });
    }
    tokens
}

fn matches_filter(f: &Filter, v: &TaskView<'_>, ctx: &Ctx) -> bool {
    let t = v.task;
    let today = ctx.today;
    match f {
        Filter::Text(s) => t.title.to_lowercase().contains(s.as_str()),
        Filter::Status(s) => match s.as_str() {
            "open" => !t.done,
            other => t.status_name() == other,
        },
        Filter::Is(kind) => match kind {
            IsKind::Done => t.done,
            IsKind::Open => !t.done,
            IsKind::Blocked => v.blocked && !t.done,
            IsKind::Overdue => !t.done && t.due.map(|d| d.date() < today).unwrap_or(false),
            IsKind::Recurring => t.repeat.is_some(),
            IsKind::Root => t.parent.is_none(),
            IsKind::Sub => t.parent.is_some(),
        },
        Filter::Tag(tag) => t.tags.iter().any(|x| x == tag),
        Filter::Priority(p) => t.priority == Some(*p),
        Filter::Due(spec) => spec.matches(t.due.map(|d| d.date()), today, !t.done),
        Filter::Start(spec) => spec.matches(t.start.map(|d| d.date()), today, !t.done),
        Filter::Completed(spec) => spec.matches(t.completed.map(|d| d.date()), today, !t.done),
        Filter::Note(s) => v.note_path.to_lowercase().contains(s.as_str()),
        Filter::Id(id) => t.id.as_deref() == Some(id.as_str()),
        Filter::Has(h) => match h {
            HasKind::Due => t.due.is_some(),
            HasKind::Start => t.start.is_some(),
            HasKind::Done => t.completed.is_some(),
            HasKind::Tag => !t.tags.is_empty(),
            HasKind::Priority => t.priority.is_some(),
            HasKind::Status => t.status.is_some(),
            HasKind::Estimate => t.estimate_minutes.is_some(),
            HasKind::Repeat => t.repeat.is_some(),
            HasKind::Id => t.id.is_some(),
        },
    }
}

/// Orders by an optional day, items without one last.
fn cmp_opt_date(a: Option<Date>, b: Option<Date>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_by(field: SortField, a: &TaskView<'_>, b: &TaskView<'_>) -> Ordering {
    match field {
        SortField::Due => cmp_opt_date(a.task.due.map(|d| d.date()), b.task.due.map(|d| d.date())),
        SortField::Start => cmp_opt_date(a.task.start.map(|d| d.date()), b.task.start.map(|d| d.date())),
        // Highest priority first; tasks without a priority come last.
        SortField::Priority => match (a.task.priority, b.task.priority) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
        SortField::Title => a.task.title.to_lowercase().cmp(&b.task.title.to_lowercase()),
        SortField::Note => a.note_path.cmp(b.note_path),
        SortField::Status => a.task.status_name().cmp(b.task.status_name()),
    }
}

/// Open tasks first, then due date, priority, note and line.
fn default_order(a: &TaskView<'_>, b: &TaskView<'_>) -> Ordering {
    a.task
        .done
        .cmp(&b.task.done)
        .then_with(|| compare_by(SortField::Due, a, b))
        .then_with(|| compare_by(SortField::Priority, a, b))
        .then_with(|| a.note_path.cmp(b.note_path))
        .then_with(|| a.task.line.cmp(&b.task.line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::tasks::parse_tasks;

    const TEXT: &str = "- [ ] Write parser [due: 2026-11-03] [priority: high] #dev ^t-p\n- [ ] Review docs [due: 2026-11-10] #docs\n- [x] Choose name [done: 2026-10-20] #dev\n- [ ] Later someday\n- [ ] Overdue thing [due: 2026-10-01] [priority: urgent]\n  - [ ] sub task [status: active]\n";

    fn today() -> Ctx {
        Ctx {
            today: Date::parse("2026-11-03").unwrap(),
        }
    }

    fn run(query: &str) -> Vec<String> {
        let doc = parse(TEXT);
        let tasks = parse_tasks(&doc).tasks;
        let q = TaskQuery::parse(query).unwrap_or_else(|e| panic!("{}: {}", query, e));
        let mut views: Vec<TaskView<'_>> = tasks
            .iter()
            .map(|t| TaskView {
                task: t,
                note_path: "a.md",
                blocked: t.title.starts_with("Review"),
            })
            .filter(|v| q.matches(v, &today()))
            .collect();
        q.sort(&mut views);
        views.iter().map(|v| v.task.title.clone()).collect()
    }

    fn set(query: &str) -> Vec<String> {
        let mut r = run(query);
        r.sort();
        r
    }

    #[test]
    fn empty_query_matches_everything_with_a_sensible_default_order() {
        let all = run("");
        assert_eq!(all.len(), 6);
        // Open tasks first; among them overdue (due earliest) first, tasks without a due date last.
        assert_eq!(all[0], "Overdue thing");
        assert_eq!(all[1], "Write parser #dev");
        assert_eq!(all[2], "Review docs #docs");
        assert_eq!(all.last().unwrap(), "Choose name #dev");
    }

    #[test]
    fn tags_text_and_negation() {
        assert_eq!(set("tag:dev"), vec!["Choose name #dev", "Write parser #dev"]);
        assert_eq!(set("tag:#dev"), set("tag:dev"));
        assert_eq!(set("-tag:dev").len(), 4);
        assert_eq!(set("parser"), vec!["Write parser #dev"]);
        assert_eq!(set("\"review docs\""), vec!["Review docs #docs"]);
        assert!(set("parser review").is_empty());
        assert_eq!(set("PARSER"), vec!["Write parser #dev"]);
        assert_eq!(set("-parser").len(), 5);
    }

    #[test]
    fn date_filters() {
        assert_eq!(set("due:today"), vec!["Write parser #dev"]);
        assert_eq!(set("due:overdue"), vec!["Overdue thing"]);
        assert_eq!(set("due:week"), vec!["Write parser #dev"]);
        assert_eq!(set("due:next7"), vec!["Review docs #docs", "Write parser #dev"]);
        assert_eq!(set("due:<2026-11-05"), vec!["Overdue thing", "Write parser #dev"]);
        assert_eq!(set("due:>today"), vec!["Review docs #docs"]);
        assert_eq!(set("due:2026-11-01..2026-11-30"), vec!["Review docs #docs", "Write parser #dev"]);
        assert_eq!(set("due:2026-11-30..2026-11-01"), set("due:2026-11-01..2026-11-30"));
        assert_eq!(set("due:none").len(), 3);
        assert_eq!(set("due:2026-11-10"), vec!["Review docs #docs"]);
        assert_eq!(set("done:2026-10-20"), vec!["Choose name #dev"]);
        assert_eq!(set("due:tomorrow"), Vec::<String>::new());
    }

    #[test]
    fn status_priority_is_has_and_ids() {
        assert_eq!(set("status:active"), vec!["sub task"]);
        assert_eq!(set("status:done"), vec!["Choose name #dev"]);
        assert_eq!(set("status:todo").len(), 4);
        assert_eq!(set("status:open").len(), 5);
        assert_eq!(set("priority:high"), vec!["Write parser #dev"]);
        assert_eq!(set("has:due").len(), 3);
        assert_eq!(set("has:priority").len(), 2);
        assert_eq!(set("has:id"), vec!["Write parser #dev"]);
        assert_eq!(set("id:t-p"), vec!["Write parser #dev"]);
        assert_eq!(set("is:sub"), vec!["sub task"]);
        assert_eq!(set("is:root").len(), 5);
        assert_eq!(set("is:blocked"), vec!["Review docs #docs"]);
        assert_eq!(set("is:overdue"), vec!["Overdue thing"]);
        assert_eq!(set("is:recurring").len(), 0);
        assert_eq!(set("note:A.MD").len(), 6);
        assert_eq!(set("note:other").len(), 0);
    }

    #[test]
    fn combined_filters() {
        assert_eq!(set("is:open tag:dev"), vec!["Write parser #dev"]);
        assert_eq!(set("due:overdue priority:urgent"), vec!["Overdue thing"]);
        assert!(set("due:overdue priority:low").is_empty());
    }

    #[test]
    fn sorting() {
        assert_eq!(run("is:open has:due sort:-due"), vec!["Review docs #docs", "Write parser #dev", "Overdue thing"]);
        assert_eq!(run("is:open has:priority sort:priority"), vec!["Overdue thing", "Write parser #dev"]);
        assert_eq!(run("is:open has:priority sort:-priority"), vec!["Write parser #dev", "Overdue thing"]);
        let by_title = run("sort:title");
        assert_eq!(by_title[0], "Choose name #dev");
        assert_eq!(by_title[1], "Later someday");
    }

    #[test]
    fn quoted_values_and_colons_in_text() {
        assert!(TaskQuery::parse("note:\"my folder\"").is_ok());
        // A word with a non-alphabetic prefix before ':' is free text.
        assert!(TaskQuery::parse("10:30").is_ok());
        assert!(TaskQuery::parse("\"colour:red\"").is_ok());
        assert_eq!(TaskQuery::parse("  due:today   ").unwrap().source(), "due:today");
    }

    #[test]
    fn errors_are_explicit() {
        for bad in ["colour:red", "due:soon", "priority:x", "is:x", "has:x", "sort:x", "tag:", "due:2026-02-30", "due:..", "status:"] {
            let e = TaskQuery::parse(bad).unwrap_err();
            assert!(!e.message.is_empty(), "{}", bad);
        }
        let e = TaskQuery::parse("colour:red").unwrap_err();
        assert!(e.message.contains("valid filters"));
    }

    #[test]
    fn never_panics() {
        for q in ["", "-", "--", "\"", "\"\"", ":", "a:", ":a", "-:", "due:<", "due:..2026-01-01", "\u{1}x", "é:ü", "sort:-", "-\"a b\""] {
            let _ = TaskQuery::parse(q);
        }
    }
}
