//! # lmf-workspace
//!
//! The *workspace index*: everything that spans more than one document. It is derived data and
//! always rebuildable from the files on disk; the files stay authoritative.
//!
//! * [`scan`] — find note files under a folder (the only module that touches the filesystem).
//! * [`index::Workspace`] — titles, link resolution, backlinks and search over in-memory notes.
//!
//! Link resolution rules are documented in `docs/adr/ADR-011-links-and-search.md`.

pub mod assets;
pub mod index;
pub mod scan;
pub mod tasks;

pub use index::{
    Backlink, Expansion, IncludeError, Note, NoteSource, Resolution, SearchHit, Workspace,
    MAX_INCLUDE_DEPTH,
};
pub use scan::{scan, ScanResult, Skipped};
pub use tasks::{write_note_atomic, DuplicateId, TaskHit};
