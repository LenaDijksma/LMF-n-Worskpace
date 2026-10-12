//! Finds note files under a folder. This is the only module in the workspace crate that touches
//! the filesystem.
//!
//! Rules: only `.md`, `.markdown` and `.lmf` files; hidden files and folders (leading `.`) are
//! skipped; symbolic links are not followed; files larger than 5 MiB or not valid UTF-8 are
//! skipped *and reported* rather than silently ignored.

use std::fs;
use std::io;
use std::path::Path;

use crate::index::NoteSource;

pub const NOTE_EXTENSIONS: &[&str] = &["md", "markdown", "lmf"];
pub const MAX_NOTE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub notes: Vec<NoteSource>,
    pub skipped: Vec<Skipped>,
}

/// Scans `root` recursively. Fails only if `root` itself cannot be read.
pub fn scan(root: &Path) -> io::Result<ScanResult> {
    let mut result = ScanResult::default();
    walk(root, root, &mut result, 0)?;
    result.notes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn walk(root: &Path, dir: &Path, result: &mut ScanResult, depth: usize) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Err(e) = walk(root, &path, result, depth + 1) {
                result.skipped.push(Skipped {
                    path: relative(root, &path),
                    reason: e.to_string(),
                });
            }
        } else if file_type.is_file() && is_note(&path) {
            let rel = relative(root, &path);
            match fs::metadata(&path) {
                Ok(m) if m.len() > MAX_NOTE_BYTES => result.skipped.push(Skipped {
                    path: rel,
                    reason: "larger than 5 MiB".to_string(),
                }),
                Ok(_) => match fs::read(&path) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(text) => result.notes.push(NoteSource { path: rel, text }),
                        Err(_) => result.skipped.push(Skipped {
                            path: rel,
                            reason: "not valid UTF-8".to_string(),
                        }),
                    },
                    Err(e) => result.skipped.push(Skipped {
                        path: rel,
                        reason: e.to_string(),
                    }),
                },
                Err(e) => result.skipped.push(Skipped {
                    path: rel,
                    reason: e.to_string(),
                }),
            }
        }
    }
    Ok(())
}

fn is_note(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| NOTE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Path relative to `root` with `/` separators.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("lmf-scan-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_notes_and_reports_skips() {
        let root = temp_dir();
        fs::create_dir_all(root.join("sub/deeper")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("a.md"), "# A").unwrap();
        fs::write(root.join("sub/b.LMF"), "b").unwrap();
        fs::write(root.join("sub/deeper/c.markdown"), "c").unwrap();
        fs::write(root.join("notes.txt"), "not a note").unwrap();
        fs::write(root.join(".hidden/h.md"), "hidden").unwrap();
        fs::write(root.join(".dot.md"), "hidden").unwrap();
        fs::write(root.join("bad.md"), [0xffu8, 0xfe, 0x00]).unwrap();

        let result = scan(&root).unwrap();
        let paths: Vec<&str> = result.notes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, vec!["a.md", "sub/b.LMF", "sub/deeper/c.markdown"]);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].path, "bad.md");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_is_an_error() {
        assert!(scan(Path::new("/definitely/not/a/real/folder")).is_err());
    }

    #[test]
    fn relative_uses_forward_slashes() {
        let root = Path::new("/r");
        assert_eq!(relative(root, Path::new("/r/a/b.md")), "a/b.md");
    }
}
