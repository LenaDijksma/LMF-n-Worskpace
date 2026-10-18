# ADR-017: Tasks

**Status:** proposed

**Context.** Spec §3.10, §6.8 and Phase 4: tasks must keep one canonical identity across views
(list, board, agenda), must not duplicate when transcluded, must not break the plain list
fallback, and dates must be explicit (date-only vs timestamp, time zones).

**Decision.**

*A task is a Markdown task-list item in a note* (`- [ ] …` / `- [x] …`), so any Markdown tool
shows a normal checkbox list. The note file is the single source of truth; the workspace index is
derived and rebuildable.

*Fields* are plain `[key: value]` text after the title, all optional:

| field | value |
|-------|-------|
| `due`, `start`, `done` | a date `2026-11-03` or a timestamp `2026-11-03T14:00+01:00` |
| `priority` | `low` `normal` `high` `urgent` |
| `status` | a column name (`active`, `review`, …); unset = `todo`; a checked box = `done` |
| `estimate` | `90m`, `2h`, `1h30m`, `1.5h` |
| `repeat` | `daily`, `weekly`, `monthly`, `yearly`, `every 2 weeks`, optional `when done` |
| `after` | ids of tasks that must be done first |

`#tags` in the title are collected; unknown `[key: value]` text stays in the title. Subtasks are
checkbox items nested by indentation. Invalid or repeated fields are warnings (`LMF021`); the line
still works as a plain task.

*Identity.* A task's stable id is its trailing block marker (`^t-parser`, ADR-013), so ids share
the document's block-id namespace (`LMF005` for repeats; `LMF022` across notes). Tasks without an
id are *provisional* (note + position). Ids are **never created silently**: `lmf task-ids` /
`taskedit::assign_ids` add them on explicit request (dry run by default), deterministically, never
colliding with existing ids. A task lives only in the note whose source contains its line;
`lmf:include` output is never indexed, so transclusion cannot create a second task.

*Dates.* Date-only values are **floating**: "that day", never shifted between time zones.
Timestamps **must** carry an explicit offset (`Z` or `±HH:MM`); a time without one is rejected as
ambiguous. Comparison and agenda grouping use the calendar day in the value's own offset. "Today"
is supplied by the caller (the app will use the local date, the CLI uses the UTC date unless
`--today` is given), so all logic is deterministic and testable.

*Edits are minimal and safe* (`lmf_core::taskedit`): checking a box, writing `[done: …]`, moving a
card (`[status: …]`), assigning an id. Each is computed as a text edit from the current note text
and refused if the task line changed since it was read; the file write re-checks the whole file
and replaces it atomically (temp file + rename). Nothing else in the note changes, so moving a
card can neither duplicate nor destroy content.

*Recurrence.* Completing an open recurring task marks it done and inserts the next occurrence on
the next line (due date advanced by the rule, from the previous due date or, with `when done`,
from the completion date; month ends are clamped; time of day and offset are kept; the copy has
no id and no status). Subtasks stay with the completed occurrence. Reopening never adds lines.

*Queries.* One small language for search, filters and saved views (`status:active tag:work
due:week sort:due`, see the module docs of `lmf_core::taskquery`); errors say what is valid.

**Rejected.** A separate task database (duplicates content, drifts from the notes); emoji or
dataview-style fields (less readable or tool-specific); silently adding ids to every task (rewrites
notes unasked); local date-times without offsets (ambiguous).

**Open.** Board and agenda views and saved views (next slice); dependency cycles; time zones for
display; reminders; editing descriptions/multi-line task bodies; import/export formats (iCalendar,
CSV); tasks inside blockquotes.
