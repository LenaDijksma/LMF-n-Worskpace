//! Images and attachments on disk: resolving references safely, size and type policy, and
//! copying files into the workspace.
//!
//! Policy (ADR-016):
//! * references are *relative paths* resolved against the folder of the note that contains them;
//!   absolute paths and paths that leave the workspace folder (including through symbolic
//!   links) are rejected;
//! * only `png`, `jpg`/`jpeg` and `gif` images are shown, at most 4 MiB and 16 megapixels (the
//!   dimensions are read from the file header *before* anything is decoded);
//! * attachments are only opened if their type is on a short allow-list (no executables,
//!   scripts, web pages or SVG).

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use lmf_core::diagnostics::{codes, Diagnostic};
use lmf_core::media::{MediaKind, MediaRef, MediaTarget};

pub const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 16_000_000;
pub const MAX_ATTACH_COPY_BYTES: u64 = 64 * 1024 * 1024;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif"];

/// Attachment types the application will hand to the system's default program.
pub const OPENABLE_EXTENSIONS: &[&str] = &[
    "pdf", "txt", "csv", "json", "rtf", "png", "jpg", "jpeg", "gif", "webp", "docx", "xlsx",
    "pptx", "odt", "ods", "odp", "epub", "zip", "mp3", "wav", "ogg", "flac", "mp4", "webm", "mov",
    "mkv",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    Absolute,
    Escapes,
    Missing,
    NotAFile,
    TooLarge { bytes: u64, max: u64 },
    UnsupportedType(String),
    TooManyPixels { width: u32, height: u32 },
    Unreadable(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Absolute => write!(
                f,
                "absolute paths are not allowed; use a path relative to this note"
            ),
            AssetError::Escapes => write!(f, "the path points outside the workspace folder"),
            AssetError::Missing => write!(f, "file not found"),
            AssetError::NotAFile => write!(f, "not a regular file"),
            AssetError::TooLarge { bytes, max } => write!(
                f,
                "file is {:.1} MiB; the limit is {} MiB",
                *bytes as f64 / 1_048_576.0,
                max / 1_048_576
            ),
            AssetError::UnsupportedType(ext) => {
                if ext.is_empty() {
                    write!(f, "unsupported file type")
                } else {
                    write!(f, "unsupported file type .{}", ext)
                }
            }
            AssetError::TooManyPixels { width, height } => write!(
                f,
                "image is {}\u{00d7}{} pixels; the limit is {} megapixels",
                width,
                height,
                MAX_IMAGE_PIXELS / 1_000_000
            ),
            AssetError::Unreadable(m) => write!(f, "cannot read file: {}", m),
        }
    }
}

/// Lower-case file extension of a path, if it has a short alphanumeric one.
pub fn extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() || ext.len() > 10 {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

pub fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext)
}

pub fn is_openable_ext(ext: &str) -> bool {
    OPENABLE_EXTENSIONS.contains(&ext)
}

/// Folder part of a workspace-relative path (`""` for top-level files).
pub fn parent_dir(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

/// Resolves `target` (decoded, relative to the note `note_rel`) to a file inside `root`.
///
/// Fails for absolute paths, paths that leave the workspace (lexically or through symbolic
/// links), missing files and non-files. The returned path is safe to read.
pub fn resolve(root: &Path, note_rel: &str, target: &str) -> Result<PathBuf, AssetError> {
    let target = target.replace('\\', "/");
    if target.starts_with('/') || target.contains(':') {
        return Err(AssetError::Absolute);
    }
    let base = parent_dir(note_rel);
    let joined = if base.is_empty() {
        target
    } else {
        format!("{}/{}", base, target)
    };
    let rel = normalize(&joined).ok_or(AssetError::Escapes)?;
    let full = root.join(&rel);

    let meta = fs::metadata(&full).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AssetError::Missing,
        _ => AssetError::Unreadable(e.to_string()),
    })?;
    if !meta.is_file() {
        return Err(AssetError::NotAFile);
    }
    // Symbolic links must not lead out of the workspace either.
    let canon = fs::canonicalize(&full).map_err(|e| AssetError::Unreadable(e.to_string()))?;
    let canon_root =
        fs::canonicalize(root).map_err(|e| AssetError::Unreadable(e.to_string()))?;
    if !canon.starts_with(&canon_root) {
        return Err(AssetError::Escapes);
    }
    Ok(full)
}

/// Resolves `.` and `..`; `None` if the path would leave the root (or is empty).
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

/// Checks an image file against the display policy and returns its pixel size. Reads the file
/// (at most [`MAX_IMAGE_BYTES`]) but decodes nothing.
pub fn check_image(path: &Path) -> Result<(u32, u32), AssetError> {
    let name = path.to_string_lossy();
    let ext = extension(&name).unwrap_or_default();
    if !is_image_ext(&ext) {
        return Err(AssetError::UnsupportedType(ext));
    }
    let meta = fs::metadata(path).map_err(|e| AssetError::Unreadable(e.to_string()))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(AssetError::TooLarge {
            bytes: meta.len(),
            max: MAX_IMAGE_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(|e| AssetError::Unreadable(e.to_string()))?;
    let (width, height) = image_dimensions(&bytes)
        .ok_or_else(|| AssetError::Unreadable("not a valid PNG, JPEG or GIF image".to_string()))?;
    if (width as u64) * (height as u64) > MAX_IMAGE_PIXELS {
        return Err(AssetError::TooManyPixels { width, height });
    }
    Ok((width, height))
}

/// Pixel size from a PNG, JPEG or GIF header, without decoding the image.
pub fn image_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    // PNG: signature, then the IHDR chunk (length, "IHDR", width, height).
    if b.len() >= 24 && b[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] && &b[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        return Some((w, h));
    }
    // GIF: "GIF87a"/"GIF89a", then little-endian 16-bit width and height.
    if b.len() >= 10 && (&b[..6] == b"GIF87a" || &b[..6] == b"GIF89a") {
        let w = u16::from_le_bytes([b[6], b[7]]) as u32;
        let h = u16::from_le_bytes([b[8], b[9]]) as u32;
        return Some((w, h));
    }
    jpeg_dimensions(b)
}

fn jpeg_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 3 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let mut marker = b[i + 1];
        while marker == 0xFF && i + 2 < b.len() {
            i += 1;
            marker = b[i + 1];
        }
        i += 2;
        match marker {
            0x01 | 0xD0..=0xD8 => continue,
            0xD9 => return None,
            _ => {}
        }
        if i + 1 >= b.len() {
            return None;
        }
        let len = ((b[i] as usize) << 8) | (b[i + 1] as usize);
        if len < 2 {
            return None;
        }
        // Start-of-frame markers carry the size.
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            if i + 6 >= b.len() {
                return None;
            }
            let h = ((b[i + 3] as u32) << 8) | (b[i + 4] as u32);
            let w = ((b[i + 5] as u32) << 8) | (b[i + 6] as u32);
            return Some((w, h));
        }
        i += len;
    }
    None
}

/// Problems with the images and attachments referenced from one note, as diagnostics.
/// Looks at the filesystem (existence, type, size) but does not decode images.
pub fn check_assets(root: &Path, note_rel: &str, refs: &[MediaRef]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for r in refs {
        match &r.target {
            MediaTarget::Remote(url) => {
                if r.kind == MediaKind::Image {
                    out.push(Diagnostic::info(
                        codes::REMOTE_IMAGE,
                        format!("remote image is not loaded (pictures from the web are never fetched): {}", url),
                        r.span,
                    ));
                }
            }
            MediaTarget::Unsupported(addr) => {
                if r.kind == MediaKind::Image {
                    out.push(Diagnostic::warning(
                        codes::REJECTED_ASSET,
                        format!("unsupported image address `{}`", addr),
                        r.span,
                    ));
                }
            }
            MediaTarget::Local(path) => match resolve(root, note_rel, path) {
                Err(AssetError::Missing) => out.push(Diagnostic::warning(
                    codes::MISSING_ASSET,
                    format!("file not found: {}", path),
                    r.span,
                )),
                Err(e) => out.push(Diagnostic::warning(
                    codes::REJECTED_ASSET,
                    format!("{}: {}", path, e),
                    r.span,
                )),
                Ok(full) => match r.kind {
                    MediaKind::Image => {
                        let ext = extension(path).unwrap_or_default();
                        let size = fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
                        if !is_image_ext(&ext) {
                            out.push(Diagnostic::warning(
                                codes::REJECTED_ASSET,
                                format!(
                                    "{}: unsupported image type .{} (supported: png, jpg, gif)",
                                    path, ext
                                ),
                                r.span,
                            ));
                        } else if size > MAX_IMAGE_BYTES {
                            out.push(Diagnostic::warning(
                                codes::REJECTED_ASSET,
                                format!(
                                    "{}: {}",
                                    path,
                                    AssetError::TooLarge { bytes: size, max: MAX_IMAGE_BYTES }
                                ),
                                r.span,
                            ));
                        }
                    }
                    MediaKind::Attachment => {
                        let ext = extension(path).unwrap_or_default();
                        if !is_openable_ext(&ext) {
                            out.push(Diagnostic::info(
                                codes::REJECTED_ASSET,
                                format!("{}: .{} files are not on the safe list and will not be opened by the app", path, ext),
                                r.span,
                            ));
                        }
                    }
                },
            },
        }
    }
    out
}

/// Path of `to` as seen from the folder `from_dir` (both workspace-relative, `/`-separated).
pub fn relative_path(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let target: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let mut common = 0usize;
    while common < from.len() && common + 1 < target.len() && from[common] == target[common] {
        common += 1;
    }
    let mut parts: Vec<&str> = Vec::new();
    for _ in common..from.len() {
        parts.push("..");
    }
    parts.extend_from_slice(&target[common..]);
    parts.join("/")
}

/// A safe file name that does not collide with anything in `dir`: unusual characters become
/// `-`, the extension is lower-cased, and `-1`, `-2`, … is appended if the name is taken.
pub fn unique_asset_name(dir: &Path, file_name: &str) -> String {
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, Some(e.to_ascii_lowercase())),
        _ => (file_name, None),
    };
    let mut clean = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            clean.push(c);
        } else if !clean.ends_with('-') {
            clean.push('-');
        }
    }
    let clean = clean.trim_matches(|c| c == '-' || c == '.').to_string();
    let stem = if clean.is_empty() { "file".to_string() } else { clean };
    let ext: Option<String> = ext.map(|e| {
        e.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(10)
            .collect::<String>()
    });
    let build = |n: u32| -> String {
        let base = if n == 0 {
            stem.clone()
        } else {
            format!("{}-{}", stem, n)
        };
        match &ext {
            Some(e) if !e.is_empty() => format!("{}.{}", base, e),
            _ => base,
        }
    };
    let mut n = 0u32;
    loop {
        let candidate = build(n);
        if !dir.join(&candidate).exists() || n >= 9999 {
            return candidate;
        }
        n += 1;
    }
}

/// Copies `source` into `<root>/assets/` (created if needed, never overwriting) and returns the
/// workspace-relative path of the copy, e.g. `assets/photo-1.png`.
pub fn attach_file(root: &Path, source: &Path) -> Result<String, String> {
    let meta = fs::metadata(source)
        .map_err(|e| format!("cannot read {}: {}", source.display(), e))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    if meta.len() > MAX_ATTACH_COPY_BYTES {
        return Err(format!(
            "{} is larger than {} MiB; not copied",
            source.display(),
            MAX_ATTACH_COPY_BYTES / 1_048_576
        ));
    }
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| "the file has no name".to_string())?;
    let dir = root.join("assets");
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create assets folder: {}", e))?;
    let unique = unique_asset_name(&dir, &name);
    fs::copy(source, dir.join(&unique)).map_err(|e| format!("cannot copy file: {}", e))?;
    Ok(format!("assets/{}", unique))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lmf_core::media::extract_media;
    use lmf_core::parse;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("lmf-assets-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        v
    }

    fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut v = b"GIF89a".to_vec();
        v.extend_from_slice(&width.to_le_bytes());
        v.extend_from_slice(&height.to_le_bytes());
        v
    }

    #[test]
    fn dimensions_from_headers() {
        assert_eq!(image_dimensions(&png(100, 50)), Some((100, 50)));
        assert_eq!(image_dimensions(&jpeg(640, 480)), Some((640, 480)));
        assert_eq!(image_dimensions(&gif(32, 16)), Some((32, 16)));
        assert_eq!(image_dimensions(b"not an image at all"), None);
        assert_eq!(image_dimensions(&[]), None);
        assert_eq!(image_dimensions(&png(1, 1)[..10]), None);
        assert_eq!(image_dimensions(&jpeg(1, 1)[..12]), None);
    }

    #[test]
    fn resolve_is_confined_to_the_workspace() {
        let root = temp_dir();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("assets/pic.png"), png(10, 10)).unwrap();
        fs::write(root.join("pic-top.png"), png(10, 10)).unwrap();

        assert!(resolve(&root, "notes/a.md", "../assets/pic.png").is_ok());
        assert!(resolve(&root, "a.md", "assets/pic.png").is_ok());
        assert!(resolve(&root, "a.md", "./assets/../pic-top.png").is_ok());
        assert_eq!(resolve(&root, "notes/a.md", "../../x.png"), Err(AssetError::Escapes));
        assert_eq!(resolve(&root, "a.md", "/etc/passwd"), Err(AssetError::Absolute));
        assert_eq!(resolve(&root, "a.md", "C:\\Windows\\x.png"), Err(AssetError::Absolute));
        assert_eq!(resolve(&root, "a.md", "assets/nope.png"), Err(AssetError::Missing));
        assert_eq!(resolve(&root, "a.md", "assets"), Err(AssetError::NotAFile));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape() {
        let base = temp_dir();
        let root = base.join("ws");
        let outside = base.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.png"), png(2, 2)).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert_eq!(resolve(&root, "a.md", "link/secret.png"), Err(AssetError::Escapes));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn image_policy() {
        let root = temp_dir();
        fs::write(root.join("ok.png"), png(1000, 1000)).unwrap();
        fs::write(root.join("huge.png"), png(5000, 5000)).unwrap();
        fs::write(root.join("fake.png"), b"just text").unwrap();
        fs::write(root.join("pic.bmp"), b"BM....").unwrap();
        let mut big = png(10, 10);
        big.resize(MAX_IMAGE_BYTES as usize + 1, 0);
        fs::write(root.join("big.png"), big).unwrap();

        assert_eq!(check_image(&root.join("ok.png")), Ok((1000, 1000)));
        assert!(matches!(
            check_image(&root.join("huge.png")),
            Err(AssetError::TooManyPixels { width: 5000, height: 5000 })
        ));
        assert!(matches!(check_image(&root.join("fake.png")), Err(AssetError::Unreadable(_))));
        assert!(matches!(check_image(&root.join("pic.bmp")), Err(AssetError::UnsupportedType(_))));
        assert!(matches!(check_image(&root.join("big.png")), Err(AssetError::TooLarge { .. })));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn diagnostics_for_references() {
        let root = temp_dir();
        fs::write(root.join("ok.png"), png(10, 10)).unwrap();
        fs::write(root.join("doc.pdf"), b"%PDF").unwrap();
        fs::write(root.join("tool.exe"), b"MZ").unwrap();
        fs::write(root.join("pic.bmp"), b"BM").unwrap();
        let src = "![a](ok.png) ![b](gone.png) ![c](https://x.org/p.png) ![d](../out.png) ![e](pic.bmp) ![f](data:x)\n\n[pdf](doc.pdf) [exe](tool.exe) [missing](nope.pdf)\n";
        let refs = extract_media(&parse(src));
        let diags = check_assets(&root, "n.md", &refs);
        let codes: Vec<&str> = diags.iter().map(|d| d.code).collect();
        assert_eq!(
            codes,
            vec![
                "LMF018", // gone.png
                "LMF020", // remote image
                "LMF019", // ../out.png leaves the workspace
                "LMF019", // pic.bmp unsupported type
                "LMF019", // data: address
                "LMF019", // tool.exe not on the safe list (info)
                "LMF018", // nope.pdf
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative_path("", "assets/a.png"), "assets/a.png");
        assert_eq!(relative_path("notes", "assets/a.png"), "../assets/a.png");
        assert_eq!(relative_path("notes/deep", "assets/a.png"), "../../assets/a.png");
        assert_eq!(relative_path("assets", "assets/a.png"), "a.png");
        assert_eq!(relative_path("notes", "notes/img/a.png"), "img/a.png");
    }

    #[test]
    fn unique_names_are_safe_and_do_not_collide() {
        let dir = temp_dir();
        assert_eq!(unique_asset_name(&dir, "My Photo (1).PNG"), "My-Photo-1.png");
        fs::write(dir.join("a.png"), b"x").unwrap();
        assert_eq!(unique_asset_name(&dir, "a.png"), "a-1.png");
        fs::write(dir.join("a-1.png"), b"x").unwrap();
        assert_eq!(unique_asset_name(&dir, "a.png"), "a-2.png");
        assert_eq!(unique_asset_name(&dir, "..."), "file");
        assert_eq!(unique_asset_name(&dir, "../../evil.sh"), "evil.sh");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn attaching_copies_without_overwriting() {
        let root = temp_dir();
        let src_dir = temp_dir();
        let src = src_dir.join("Report Final.pdf");
        fs::write(&src, b"%PDF-1").unwrap();
        let first = attach_file(&root, &src).unwrap();
        assert_eq!(first, "assets/Report-Final.pdf");
        let second = attach_file(&root, &src).unwrap();
        assert_eq!(second, "assets/Report-Final-1.pdf");
        assert_eq!(fs::read(root.join(&first)).unwrap(), b"%PDF-1");
        assert!(attach_file(&root, &src_dir.join("missing.pdf")).is_err());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn allow_lists() {
        assert!(is_openable_ext("pdf"));
        for bad in ["exe", "sh", "bat", "html", "svg", "app", "jar", "js", "desktop", "ps1"] {
            assert!(!is_openable_ext(bad), "{}", bad);
        }
        assert!(is_image_ext("png") && !is_image_ext("svg") && !is_image_ext("webp"));
        assert_eq!(extension("a/B.PDF"), Some("pdf".to_string()));
        assert_eq!(extension("noext"), None);
        assert_eq!(parent_dir("a/b/c.md"), "a/b");
        assert_eq!(parent_dir("c.md"), "");
    }
}
