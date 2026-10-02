use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Write a file atomically: write to a sibling temp file, fsync, then rename over the target.
/// A crash mid-write never leaves a truncated file behind.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    // Keep the permissions of an existing file (e.g. executable scripts).
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Write a file readable only by the current user (used for the API key).
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    atomic_write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Heuristic binary check: a NUL byte in the first 8 KiB.
pub fn looks_binary(bytes: &[u8]) -> bool {
    let n = bytes.len().min(8192);
    memchr::memchr(0, &bytes[..n]).is_some()
}

/// Truncate a string to at most `max` bytes on a char boundary, appending a marker if cut.
pub fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let mut head = half;
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len() - half;
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    let omitted = tail - head;
    format!("{}\n\n[... {omitted} bytes omitted ...]\n\n{}", &s[..head], &s[tail..])
}

/// Truncate to `max` bytes on a char boundary (keeps the start).
pub fn truncate_end(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Convert a char index into a UTF-16 code unit index (what JavaScript strings use).
pub fn char_to_utf16_index(s: &str, char_idx: usize) -> usize {
    s.chars().take(char_idx).map(char::len_utf16).sum()
}

/// Convert a byte offset into a UTF-16 code unit index.
pub fn byte_to_utf16_index(s: &str, byte_idx: usize) -> usize {
    s[..byte_idx.min(s.len())].encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        // no temp files left behind
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let s = "ééééééééééé"; // 2 bytes per char
        let t = truncate_end(s, 5);
        assert_eq!(t, "éé");
        let m = truncate_middle(&"x".repeat(100), 20);
        assert!(m.contains("bytes omitted"));
    }

    #[test]
    fn utf16_indices() {
        assert_eq!(char_to_utf16_index("a😀b", 2), 3);
        assert_eq!(byte_to_utf16_index("a😀b", 5), 3);
    }
}
