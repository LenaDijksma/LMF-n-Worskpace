//! In-memory workspace index. Pure: no filesystem access, so it is easy to test.

use std::collections::HashMap;
use std::fmt;

use lmf_core::diagnostics::Diagnostic;
use lmf_core::frontmatter::{FrontMatter, MetaValue};
use lmf_core::links::{extract_links, Link, LinkKind};
use lmf_core::parse;
use lmf_core::span::Span;
use lmf_core::tasks::{parse_tasks, Task};
use lmf_core::structure::{block_ids, block_text, find_heading, headings as doc_headings, section_span};

const NOTE_EXTENSIONS: &[&str] = &[".md", ".markdown", ".lmf"];

/// Raw input: a note's path relative to the workspace root (`/` separators) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSource {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub path: String,
    /// Front matter `title`, else the first level-1 heading, else the file name.
    pub title: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub headings: Vec<String>,
    /// Block ids (`^id`) defined in the note, plus ids of extension blocks.
    pub block_ids: Vec<String>,
    /// Tasks in this note, in source order (`Task::parent` indexes this list).
    pub tasks: Vec<Task>,
    /// Problems with task fields in this note.
    pub task_diagnostics: Vec<Diagnostic>,
    /// Outgoing links as written (unresolved).
    pub links: Vec<Link>,
    pub text: String,
    text_lower: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved(usize),
    /// Several notes match equally well; the link needs disambiguating (e.g. with a path).
    Ambiguous(Vec<usize>),
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    /// Index of the linking note.
    pub from: usize,
    pub link: Link,
    /// The line of the linking note that contains the link.
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub note: usize,
    pub score: u32,
    /// First matching body line (may be empty if only the title/path matched).
    pub snippet: String,
    /// 1-based line number of the snippet, or 0.
    pub line: usize,
}

/// Maximum nesting of includes (a note including a note including a note …).
pub const MAX_INCLUDE_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// Index of the included note.
    pub note: usize,
    pub path: String,
    pub title: String,
    /// The included text (LMF/Markdown), without the `^id` marker for block includes.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncludeError {
    MissingDocument(String),
    Ambiguous(Vec<String>),
    MissingSection(String),
    MissingBlock(String),
    /// The include would (indirectly) include itself; the payload shows the chain.
    Cycle(String),
    TooDeep,
}

impl fmt::Display for IncludeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IncludeError::MissingDocument(d) => write!(f, "document \"{}\" not found", d),
            IncludeError::Ambiguous(paths) => {
                write!(f, "document name is ambiguous ({})", paths.join(", "))
            }
            IncludeError::MissingSection(s) => write!(f, "section \"{}\" not found", s),
            IncludeError::MissingBlock(b) => write!(f, "block \"{}\" not found", b),
            IncludeError::Cycle(chain) => write!(f, "include cycle: {}", chain),
            IncludeError::TooDeep => write!(
                f,
                "includes are nested more than {} levels deep",
                MAX_INCLUDE_DEPTH
            ),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    notes: Vec<Note>,
    by_path: HashMap<String, usize>,
    by_path_lower: HashMap<String, usize>,
    backlinks: Vec<Vec<Backlink>>,
}

impl Workspace {
    pub fn build(sources: Vec<NoteSource>) -> Workspace {
        let mut ws = Workspace::default();
        for s in sources {
            ws.notes.push(build_note(s));
        }
        ws.rebuild();
        ws
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn note_index(&self, path: &str) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// Adds a note or replaces the one with the same path, then refreshes all derived data.
    pub fn upsert_note(&mut self, source: NoteSource) {
        let note = build_note(source);
        match self.by_path.get(&note.path).copied() {
            Some(i) => self.notes[i] = note,
            None => {
                self.notes.push(note);
                self.notes.sort_by(|a, b| a.path.cmp(&b.path));
            }
        }
        self.rebuild();
    }

    pub fn backlinks(&self, note: usize) -> &[Backlink] {
        self.backlinks.get(note).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Outgoing links of a note together with how each resolves.
    pub fn outgoing(&self, note: usize) -> Vec<(Link, Resolution)> {
        match self.notes.get(note) {
            Some(n) => n
                .links
                .iter()
                .map(|l| (l.clone(), self.resolve(note, l)))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Whether the heading or block named by the link fragment exists in `target`.
    pub fn fragment_exists(&self, target: usize, link: &Link) -> bool {
        use lmf_core::links::Fragment;
        match (&link.fragment, self.notes.get(target)) {
            (Fragment::None, _) => true,
            (Fragment::Block(id), Some(n)) => n.block_ids.iter().any(|b| b == id),
            (Fragment::Block(_), None) => false,
            (Fragment::Heading(h), Some(n)) => {
                let h = h.to_lowercase();
                n.headings.iter().any(|x| x.to_lowercase() == h)
            }
            (Fragment::Heading(_), None) => false,
        }
    }

    // --- resolution --------------------------------------------------------------------

    pub fn resolve(&self, from: usize, link: &Link) -> Resolution {
        if link.target.is_empty() {
            return Resolution::Resolved(from);
        }
        match link.kind {
            // External links are not part of the workspace graph.
            LinkKind::External => Resolution::Unresolved,
            LinkKind::Markdown => self.resolve_path(from, &link.target),
            LinkKind::Wiki => {
                if link.target.contains('/') || has_note_extension(&link.target) {
                    match self.resolve_path(from, &link.target) {
                        Resolution::Unresolved => self.resolve_name(from, strip_extension(&link.target)),
                        r => r,
                    }
                } else {
                    self.resolve_name(from, &link.target)
                }
            }
        }
    }

    fn resolve_path(&self, from: usize, target: &str) -> Resolution {
        let base_dir = self
            .notes
            .get(from)
            .map(|n| dir_of(&n.path))
            .unwrap_or_default();
        let joined = if base_dir.is_empty() {
            target.to_string()
        } else {
            format!("{}/{}", base_dir, target)
        };
        let mut candidates: Vec<String> = Vec::new();
        if let Some(p) = normalize(&joined) {
            candidates.push(p);
        }
        if let Some(p) = normalize(target) {
            candidates.push(p);
        }
        for cand in candidates {
            let mut variants = vec![cand.clone()];
            if !has_note_extension(&cand) {
                for ext in NOTE_EXTENSIONS {
                    variants.push(format!("{}{}", cand, ext));
                }
            }
            for v in variants {
                if let Some(i) = self.by_path_lower.get(&v.to_lowercase()) {
                    return Resolution::Resolved(*i);
                }
            }
        }
        Resolution::Unresolved
    }

    /// Name resolution tiers: file name, then title, then alias (all case-insensitive).
    /// Ambiguity is broken in favour of the single candidate in the linking note's folder.
    fn resolve_name(&self, from: usize, name: &str) -> Resolution {
        let wanted = name.trim().to_lowercase();
        if wanted.is_empty() {
            return Resolution::Unresolved;
        }
        for tier in 0..3usize {
            let found: Vec<usize> = self
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| match tier {
                    0 => file_stem(&n.path).to_lowercase() == wanted,
                    1 => n.title.to_lowercase() == wanted,
                    _ => n.aliases.iter().any(|a| a.to_lowercase() == wanted),
                })
                .map(|(i, _)| i)
                .collect();
            match found.len() {
                0 => continue,
                1 => return Resolution::Resolved(found[0]),
                _ => {
                    let from_dir = self.notes.get(from).map(|n| dir_of(&n.path)).unwrap_or_default();
                    let same_dir: Vec<usize> = found
                        .iter()
                        .copied()
                        .filter(|&i| dir_of(&self.notes[i].path) == from_dir)
                        .collect();
                    if same_dir.len() == 1 {
                        return Resolution::Resolved(same_dir[0]);
                    }
                    return Resolution::Ambiguous(found);
                }
            }
        }
        Resolution::Unresolved
    }

    fn rebuild(&mut self) {
        self.by_path.clear();
        self.by_path_lower.clear();
        for (i, n) in self.notes.iter().enumerate() {
            self.by_path.insert(n.path.clone(), i);
            self.by_path_lower.insert(n.path.to_lowercase(), i);
        }
        let mut backlinks: Vec<Vec<Backlink>> = vec![Vec::new(); self.notes.len()];
        for from in 0..self.notes.len() {
            for link in &self.notes[from].links {
                if let Resolution::Resolved(to) = self.resolve(from, link) {
                    if to != from {
                        backlinks[to].push(Backlink {
                            from,
                            link: link.clone(),
                            context: context_of(&self.notes[from].text, link.span),
                        });
                    }
                }
            }
        }
        self.backlinks = backlinks;
    }

    // --- transclusion ------------------------------------------------------------------

    /// Expands an `lmf:include`: finds the target document and returns the requested part
    /// (a section, a block, or the whole body without front matter) as LMF/Markdown text.
    ///
    /// `chain` lists the notes currently being expanded, outermost first; the last entry is the
    /// note that contains the include (its folder and aliases are used to resolve `document`).
    /// Cycles and excessive depth are reported as errors, never followed.
    pub fn expand_include(
        &self,
        chain: &[usize],
        document: &str,
        section: Option<&str>,
        block: Option<&str>,
    ) -> Result<Expansion, IncludeError> {
        if chain.len() > MAX_INCLUDE_DEPTH {
            return Err(IncludeError::TooDeep);
        }
        if self.notes.is_empty() || document.trim().is_empty() {
            return Err(IncludeError::MissingDocument(document.to_string()));
        }
        let from = chain.last().copied().unwrap_or(0);
        let link = Link {
            kind: LinkKind::Wiki,
            target: document.to_string(),
            fragment: lmf_core::links::Fragment::None,
            label: None,
            span: Span::new(0, 0),
        };
        let target = match self.resolve(from, &link) {
            Resolution::Resolved(i) => i,
            Resolution::Ambiguous(c) => {
                return Err(IncludeError::Ambiguous(
                    c.iter().map(|&i| self.notes[i].path.clone()).collect(),
                ))
            }
            Resolution::Unresolved => {
                return Err(IncludeError::MissingDocument(document.to_string()))
            }
        };
        if chain.contains(&target) {
            let mut names: Vec<String> = chain
                .iter()
                .filter_map(|&i| self.notes.get(i).map(|n| n.title.clone()))
                .collect();
            names.push(self.notes[target].title.clone());
            return Err(IncludeError::Cycle(names.join(" \u{2192} ")));
        }

        let note = &self.notes[target];
        let doc = parse(&note.text);
        let section = section.map(|s| s.trim()).filter(|s| !s.is_empty());
        let block = block.map(|b| b.trim()).filter(|b| !b.is_empty());
        let text = if let Some(sec) = section {
            let outline = doc_headings(&doc);
            let i = find_heading(&outline, sec)
                .ok_or_else(|| IncludeError::MissingSection(sec.to_string()))?;
            doc.text(section_span(&doc, &outline, i)).replace("\r\n", "\n")
        } else if let Some(id) = block {
            let ids = block_ids(&doc);
            let found = ids
                .iter()
                .find(|b| b.id == id)
                .ok_or_else(|| IncludeError::MissingBlock(id.to_string()))?;
            block_text(&doc, found)
        } else {
            let start = doc.front_matter.as_ref().map(|fm| fm.span.end).unwrap_or(0);
            doc.source
                .get(start..)
                .unwrap_or("")
                .trim()
                .replace("\r\n", "\n")
        };
        Ok(Expansion {
            note: target,
            path: note.path.clone(),
            title: note.title.clone(),
            text,
        })
    }

    // --- search ------------------------------------------------------------------------

    /// All whitespace-separated terms must match (case-insensitively) somewhere in a note's
    /// title, path, aliases, tags, headings or text. Results are ranked: title > alias/tag/
    /// heading > path > body occurrences.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = Vec::new();
        for (idx, note) in self.notes.iter().enumerate() {
            let title = note.title.to_lowercase();
            let path = note.path.to_lowercase();
            let aliases = note.aliases.join("\n").to_lowercase();
            let tags = note.tags.join("\n").to_lowercase();
            let headings = note.headings.join("\n").to_lowercase();

            let mut score = 0u32;
            let mut all = true;
            for t in &terms {
                let in_title = title.contains(t.as_str());
                let in_alias = aliases.contains(t.as_str());
                let in_tag = tags.contains(t.as_str());
                let in_heading = headings.contains(t.as_str());
                let in_path = path.contains(t.as_str());
                let body_count = note.text_lower.matches(t.as_str()).count();
                if !(in_title || in_alias || in_tag || in_heading || in_path || body_count > 0) {
                    all = false;
                    break;
                }
                if title == *t {
                    score += 100;
                } else if in_title {
                    score += 50;
                }
                if in_alias {
                    score += 30;
                }
                if in_tag {
                    score += 20;
                }
                if in_heading {
                    score += 20;
                }
                if in_path {
                    score += 10;
                }
                score += body_count.min(10) as u32 * 2;
            }
            if !all {
                continue;
            }
            let (snippet, line) = first_matching_line(&note.text, &terms[0]);
            hits.push(SearchHit {
                note: idx,
                score,
                snippet,
                line,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| self.notes[a.note].title.cmp(&self.notes[b.note].title))
                .then_with(|| self.notes[a.note].path.cmp(&self.notes[b.note].path))
        });
        hits.truncate(limit);
        hits
    }
}

// --- helpers --------------------------------------------------------------------------------

fn build_note(src: NoteSource) -> Note {
    let doc = parse(&src.text);
    let stem = file_stem(&src.path).to_string();
    let outline = doc_headings(&doc);

    let fm_title = doc
        .front_matter
        .as_ref()
        .and_then(|fm| fm.get("title"))
        .and_then(|e| match &e.value {
            MetaValue::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        });
    let headings: Vec<String> = outline.iter().map(|h| h.text.clone()).collect();
    let first_h1 = outline
        .iter()
        .find(|h| h.level == 1 && !h.text.is_empty())
        .map(|h| h.text.clone());
    let mut ids: Vec<String> = block_ids(&doc).into_iter().map(|b| b.id).collect();
    for d in doc.directives() {
        if let Some(a) = d.attr_str("id") {
            ids.push(a.to_string());
        }
    }
    let title = fm_title.or(first_h1).unwrap_or(stem);

    let (aliases, tags) = match &doc.front_matter {
        Some(fm) => (meta_list(fm, "aliases"), meta_list(fm, "tags")),
        None => (Vec::new(), Vec::new()),
    };
    let links = extract_links(&doc);
    let task_parse = parse_tasks(&doc);
    let text_lower = src.text.to_lowercase();
    Note {
        path: src.path,
        title,
        aliases,
        tags,
        headings,
        block_ids: ids,
        tasks: task_parse.tasks,
        task_diagnostics: task_parse.diagnostics,
        links,
        text: src.text,
        text_lower,
    }
}

fn meta_list(fm: &FrontMatter, key: &str) -> Vec<String> {
    match fm.get(key).map(|e| &e.value) {
        Some(MetaValue::List(items)) => items
            .iter()
            .filter_map(|i| match i {
                MetaValue::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            })
            .collect(),
        Some(MetaValue::Str(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

fn has_note_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    NOTE_EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

fn strip_extension(path: &str) -> &str {
    let lower = path.to_lowercase();
    for ext in NOTE_EXTENSIONS {
        if lower.ends_with(ext) {
            return &path[..path.len() - ext.len()];
        }
    }
    path
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    strip_extension(name)
}

fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

/// Resolves `.` and `..` segments. Returns `None` if the path escapes the workspace root.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn context_of(text: &str, span: Span) -> String {
    let start = span.start.min(text.len());
    let line_start = text.get(..start).and_then(|t| t.rfind('\n')).map(|i| i + 1).unwrap_or(0);
    let line_end = text
        .get(start..)
        .and_then(|t| t.find('\n'))
        .map(|i| start + i)
        .unwrap_or(text.len());
    let line = text.get(line_start..line_end).unwrap_or("").trim();
    truncate_chars(line, 160)
}

fn first_matching_line(text: &str, term: &str) -> (String, usize) {
    for (i, line) in text.lines().enumerate() {
        if line.to_lowercase().contains(term) {
            return (truncate_chars(line.trim(), 140), i + 1);
        }
    }
    (String::new(), 0)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}\u{2026}", cut.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn idx(w: &Workspace, path: &str) -> usize {
        w.note_index(path).expect(path)
    }

    fn sample() -> Workspace {
        ws(&[
            ("a.md", "# Alpha\n\nLinks to [[Beta]] and [[Missing]].\n"),
            ("b.md", "---\ntitle: Beta\naliases: [Bee]\ntags: [greek]\n---\n\nBack to [[alpha]] and [[Bee]].\n"),
            ("sub/c.md", "[up](../a.md) and [[Beta#Intro]]\n"),
        ])
    }

    #[test]
    fn titles() {
        let w = sample();
        assert_eq!(w.notes()[idx(&w, "a.md")].title, "Alpha");
        assert_eq!(w.notes()[idx(&w, "b.md")].title, "Beta");
        assert_eq!(w.notes()[idx(&w, "sub/c.md")].title, "c");
    }

    #[test]
    fn wikilinks_resolve_by_name_title_and_alias() {
        let w = sample();
        let a = idx(&w, "a.md");
        let b = idx(&w, "b.md");
        let out = w.outgoing(a);
        assert_eq!(out[0].1, Resolution::Resolved(b));
        assert_eq!(out[1].1, Resolution::Unresolved);
        let out_b = w.outgoing(b);
        assert_eq!(out_b[0].1, Resolution::Resolved(a));
        assert_eq!(out_b[1].1, Resolution::Resolved(b));
    }

    #[test]
    fn markdown_links_resolve_relative_paths() {
        let w = sample();
        let c = idx(&w, "sub/c.md");
        let out = w.outgoing(c);
        assert_eq!(out[0].1, Resolution::Resolved(idx(&w, "a.md")));
    }

    #[test]
    fn backlinks_exclude_self_links_and_carry_context() {
        let w = sample();
        let b = idx(&w, "b.md");
        let back = w.backlinks(b);
        let from: Vec<usize> = back.iter().map(|x| x.from).collect();
        assert_eq!(from.len(), 2);
        assert!(from.contains(&idx(&w, "a.md")));
        assert!(from.contains(&idx(&w, "sub/c.md")));
        assert!(back.iter().any(|x| x.context.contains("Links to")));
    }

    #[test]
    fn same_name_is_ambiguous_unless_same_folder() {
        let w = ws(&[
            ("x/Plan.md", "# Plan X\n"),
            ("y/Plan.md", "# Plan Y\n"),
            ("x/Other.md", "see [[Plan]]\n"),
            ("z.md", "see [[Plan]] or [[y/Plan]]\n"),
        ]);
        let other = idx(&w, "x/Other.md");
        assert_eq!(w.outgoing(other)[0].1, Resolution::Resolved(idx(&w, "x/Plan.md")));
        let z = idx(&w, "z.md");
        let out = w.outgoing(z);
        assert!(matches!(out[0].1, Resolution::Ambiguous(ref v) if v.len() == 2));
        assert_eq!(out[1].1, Resolution::Resolved(idx(&w, "y/Plan.md")));
    }

    #[test]
    fn links_are_case_insensitive_and_paths_cannot_escape() {
        let w = ws(&[("a.md", "[[BETA]] [x](../../etc/b.md)\n"), ("beta.md", "x\n")]);
        let out = w.outgoing(idx(&w, "a.md"));
        assert_eq!(out[0].1, Resolution::Resolved(idx(&w, "beta.md")));
        assert_eq!(out[1].1, Resolution::Unresolved);
    }

    #[test]
    fn heading_fragments_are_checked() {
        let w = ws(&[("a.md", "[[b#Intro]] [[b#Nope]]\n"), ("b.md", "# B\n\n## Intro\n")]);
        let a = idx(&w, "a.md");
        let b = idx(&w, "b.md");
        let out = w.outgoing(a);
        assert!(w.fragment_exists(b, &out[0].0));
        assert!(!w.fragment_exists(b, &out[1].0));
    }

    #[test]
    fn search_requires_all_terms_and_ranks_titles_first() {
        let w = sample();
        let hits = w.search("beta", 10);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].note, idx(&w, "b.md"));
        let hits = w.search("missing links", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note, idx(&w, "a.md"));
        assert!(hits[0].snippet.contains("Missing") || hits[0].snippet.contains("Links"));
        assert!(w.search("zzzz", 10).is_empty());
        assert!(w.search("   ", 10).is_empty());
    }

    #[test]
    fn search_finds_tags_and_aliases() {
        let w = sample();
        assert_eq!(w.search("greek", 10)[0].note, idx(&w, "b.md"));
        assert!(w.search("bee", 10).iter().any(|h| h.note == idx(&w, "b.md")));
    }

    #[test]
    fn upsert_refreshes_backlinks() {
        let mut w = sample();
        let c = idx(&w, "sub/c.md");
        assert!(w.backlinks(c).is_empty());
        w.upsert_note(NoteSource {
            path: "a.md".to_string(),
            text: "# Alpha\n\nnow links [[c]]\n".to_string(),
        });
        let c = idx(&w, "sub/c.md");
        assert_eq!(w.backlinks(c).len(), 1);
        w.upsert_note(NoteSource {
            path: "new.md".to_string(),
            text: "[[Alpha]]".to_string(),
        });
        let a = idx(&w, "a.md");
        // b.md ([[alpha]]), sub/c.md ([up](../a.md)) and new.md ([[Alpha]])
        assert_eq!(w.backlinks(a).len(), 3);
    }

    fn include_ws() -> Workspace {
        ws(&[
            ("a.md", "# A\n\n:::lmf:include {document=\"b\" section=\"Milestones\"}\nfallback\n:::\n"),
            (
                "b.md",
                "---\ntitle: B\n---\n\n# B\n\n## Milestones\n\nM1 ^m1\n\n### Sub\n\ndeep\n\n## Other\n\nnope\n",
            ),
            ("c.md", ":::lmf:include {document=\"d\"}\n:::\n"),
            ("d.md", ":::lmf:include {document=\"c\"}\n:::\n"),
            ("x/Plan.md", "x\n"),
            ("y/Plan.md", "y\n"),
        ])
    }

    #[test]
    fn include_section_block_and_whole_document() {
        let w = include_ws();
        let a = idx(&w, "a.md");
        let sec = w.expand_include(&[a], "b", Some("Milestones"), None).unwrap();
        assert_eq!(sec.path, "b.md");
        assert!(sec.text.starts_with("## Milestones"));
        assert!(sec.text.contains("M1") && sec.text.contains("### Sub") && sec.text.contains("deep"));
        assert!(!sec.text.contains("nope"));

        let blk = w.expand_include(&[a], "b", None, Some("m1")).unwrap();
        assert_eq!(blk.text, "M1");

        let whole = w.expand_include(&[a], "B", None, None).unwrap();
        assert!(whole.text.starts_with("# B"));
        assert!(whole.text.contains("nope"));
        assert!(!whole.text.contains("title: B"));
    }

    #[test]
    fn include_errors() {
        let w = include_ws();
        let a = idx(&w, "a.md");
        assert_eq!(
            w.expand_include(&[a], "b", Some("Nope"), None),
            Err(IncludeError::MissingSection("Nope".to_string()))
        );
        assert_eq!(
            w.expand_include(&[a], "b", None, Some("zz")),
            Err(IncludeError::MissingBlock("zz".to_string()))
        );
        assert_eq!(
            w.expand_include(&[a], "ghost", None, None),
            Err(IncludeError::MissingDocument("ghost".to_string()))
        );
        assert!(matches!(
            w.expand_include(&[a], "Plan", None, None),
            Err(IncludeError::Ambiguous(ref p)) if p.len() == 2
        ));
    }

    #[test]
    fn include_cycles_and_depth_are_stopped() {
        let w = include_ws();
        let c = idx(&w, "c.md");
        let d = idx(&w, "d.md");
        // c includes d (fine), d includes c while c is already being expanded: cycle.
        assert!(w.expand_include(&[c], "d", None, None).is_ok());
        let err = w.expand_include(&[c, d], "c", None, None).unwrap_err();
        assert!(matches!(err, IncludeError::Cycle(_)));
        assert!(err.to_string().contains("cycle"));
        // A note including itself.
        assert!(matches!(
            w.expand_include(&[c], "c", None, None),
            Err(IncludeError::Cycle(_))
        ));
        // Excessive depth.
        let deep = vec![c; MAX_INCLUDE_DEPTH + 1];
        assert_eq!(w.expand_include(&deep, "d", None, None), Err(IncludeError::TooDeep));
    }

    #[test]
    fn block_fragments_are_checked() {
        let w = ws(&[("a.md", "[[b#block:m1]] [[b#block:nope]]\n"), ("b.md", "Text ^m1\n")]);
        let a = idx(&w, "a.md");
        let b = idx(&w, "b.md");
        let out = w.outgoing(a);
        assert!(w.fragment_exists(b, &out[0].0));
        assert!(!w.fragment_exists(b, &out[1].0));
    }

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize("a/./b/../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(normalize("../x.md"), None);
        assert_eq!(normalize(""), None);
    }

    #[test]
    fn truncation_is_char_safe() {
        let s = "é".repeat(300);
        let t = truncate_chars(&s, 10);
        assert_eq!(t.chars().count(), 11);
    }
}
