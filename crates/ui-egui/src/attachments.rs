//! Saving an attachment to the Downloads folder. Nothing is opened or run: the file is
//! written under a name cleaned of paths and control characters, never over an existing one,
//! and the status bar says where it went.

use std::path::{Path, PathBuf};

/// Longest file name written, in characters.
const MAX_NAME: usize = 120;

/// `name` as a plain file name: no directories, no control characters, no leading dot, no
/// Windows-reserved characters, bounded; `bijlage` when nothing is left.
pub fn safe_filename(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut cleaned: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .chars()
        .take(MAX_NAME)
        .collect();
    cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    let stem = cleaned.rsplit('.').next_back().unwrap_or("").to_ascii_uppercase();
    if cleaned.is_empty() || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || stem.starts_with("COM") || stem.starts_with("LPT") {
        return "bijlage".into();
    }
    cleaned
}

/// Writes `bytes` into `dir` as `name`, adding ` (2)`, ` (3)`… when the name is taken. The
/// file is created new, so nothing existing is ever replaced.
pub fn save_to(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let name = safe_filename(name);
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    for n in 1..1000 {
        let candidate = if n == 1 { name.clone() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(candidate);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many files with this name"))
}

/// The person's Downloads folder, else their home folder.
pub fn downloads_dir() -> Option<PathBuf> {
    let dirs = directories::UserDirs::new()?;
    dirs.download_dir().map(Path::to_path_buf).or_else(|| Some(dirs.home_dir().to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_paths_and_control_characters() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("C:\\Users\\x\\factuur.pdf"), "factuur.pdf");
        assert_eq!(safe_filename(".bashrc"), "bashrc");
        assert_eq!(safe_filename("a\u{7}b:c?.txt"), "ab_c_.txt");
        assert_eq!(safe_filename("   "), "bijlage");
        assert_eq!(safe_filename("CON.txt"), "bijlage");
        assert_eq!(safe_filename("verslag."), "verslag");
        assert_eq!(safe_filename(&"x".repeat(300)).chars().count(), MAX_NAME);
    }

    #[test]
    fn saving_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("mailcraft-att-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = save_to(&dir, "verslag.pdf", b"one").unwrap();
        let b = save_to(&dir, "verslag.pdf", b"two").unwrap();
        let c = save_to(&dir, "../verslag.pdf", b"three").unwrap();
        assert_eq!(a.file_name().unwrap(), "verslag.pdf");
        assert_eq!(b.file_name().unwrap(), "verslag (2).pdf");
        assert_eq!(c.file_name().unwrap(), "verslag (3).pdf");
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(c.parent().unwrap(), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
