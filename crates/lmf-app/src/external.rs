//! Opening external (web) links: validated, never automatic, never through a shell.
//!
//! The preview never loads or executes remote content. A link is only opened after the user
//! confirms, and only if it is a plain `http://` or `https://` URL.

use std::path::Path;
use std::process::Command;

const MAX_URL_LEN: usize = 2048;

/// True for `http(s)://host...` URLs without whitespace or control characters.
pub fn is_safe_url(url: &str) -> bool {
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return false;
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return false;
    };
    // Needs a host: at least one character before any path/query, and no credentials.
    let host = rest.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("");
    !host.is_empty() && !host.contains('@')
}

/// Hands the URL to the operating system's default handler. The URL is passed as a single
/// argument (no shell), after validation.
pub fn open(url: &str) -> Result<(), String> {
    if !is_safe_url(url) {
        return Err("blocked: not a plain http(s) link".to_string());
    }
    let result = if cfg!(target_os = "windows") {
        Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| format!("could not open the link: {}", e))
}

/// Hands a local file to the operating system's default program. The caller must already have
/// vetted the path (inside the workspace, allow-listed type) and asked the user.
pub fn open_path(path: &Path) -> Result<(), String> {
    let result = if cfg!(target_os = "windows") {
        Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(path)
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(path).spawn()
    } else {
        Command::new("xdg-open").arg(path).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| format!("could not open the file: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_web_urls() {
        assert!(is_safe_url("https://example.org/a?b=1#c"));
        assert!(is_safe_url("HTTP://Example.org"));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://x.org",
            "https://",
            "https:///path",
            "https://user:pw@host.org",
            "https://exa mple.org",
            "https://example.org/\n",
            "//example.org",
            "mailto:a@b.c",
        ] {
            assert!(!is_safe_url(bad), "{:?}", bad);
        }
        assert!(!is_safe_url(&format!("https://x.org/{}", "a".repeat(3000))));
    }
}
