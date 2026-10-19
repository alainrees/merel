//! Saving an attachment to the Downloads folder, and the preview of a picture. Nothing is
//! opened or run: the file is written under a name cleaned of paths and control characters,
//! never over an existing one, and the status bar says where it went. A picture attachment
//! gets a thumbnail under the message and a window with the full picture, decoded by the same
//! image loader the HTML view uses; other kinds are only saved.

use std::path::{Path, PathBuf};

use mailcraft_mime::Attachment;

use crate::{MailApp, Tokens};

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

/// Largest picture Mailcraft decodes for a preview, in bytes; bigger ones are only saved.
pub const MAX_PREVIEW: usize = 8 * 1024 * 1024;

/// Whether an attachment is a picture Mailcraft shows a preview of, as Outlook does: one of
/// the formats the decoder knows, not empty and not too big. Anything else is only saved.
pub fn previewable(a: &Attachment) -> bool {
    matches!(a.mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp")
        && !a.bytes.is_empty()
        && a.bytes.len() <= MAX_PREVIEW
}

/// The picture open in the preview window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub attachment: Attachment,
    /// The id the image loader caches the decoded picture under.
    pub uri: String,
}

/// The loader id of attachment `index` of message `message_id`; the message id keeps two
/// attachments with the same name apart.
pub fn preview_uri(message_id: &str, index: usize) -> String {
    format!("bytes://attachment/{message_id}/{index}")
}

/// What the thumbnail and the window are called, for the accessibility tree and the hover text.
pub fn preview_label(name: &str) -> String {
    format!("Voorbeeld van {name}")
}

impl MailApp {
    /// Saves the attachment under Downloads and says in the status bar where it went.
    pub fn save_attachment(&mut self, a: &Attachment) {
        self.notice = Some(match downloads_dir() {
            Some(dir) => match save_to(&dir, &a.name, &a.bytes) {
                Ok(path) => format!("Bijlage opgeslagen als {}", path.display()),
                Err(e) => format!("Bijlage niet opgeslagen: {e}"),
            },
            None => "Bijlage niet opgeslagen: geen map Downloads gevonden.".into(),
        });
    }
}

/// The preview window, while a picture is open: the picture fitted to the window, with
/// "Opslaan in Downloads" and "Sluiten".
pub fn preview_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(p) = app.preview.clone() else { return };
    let mut save = false;
    let mut close = false;
    let screen = ctx.content_rect().size();
    let max = egui::vec2((screen.x - 80.0).clamp(200.0, 1000.0), (screen.y - 160.0).clamp(150.0, 700.0));
    egui::Window::new(preview_label(&p.attachment.name)).collapsible(false).resizable(false).show(ctx, |ui| {
        let image =
            egui::Image::from_bytes(p.uri.clone(), egui::load::Bytes::Shared(p.attachment.bytes.clone())).max_size(max).fit_to_original_size(1.0);
        ui.add(image);
        ui.label(egui::RichText::new(format!("{}  ({})", p.attachment.name, crate::size_text(p.attachment.size))).size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Opslaan in Downloads").clicked() {
                save = true;
            }
            if ui.button("Sluiten").clicked() {
                close = true;
            }
        });
    });
    if save {
        app.save_attachment(&p.attachment);
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.preview = None;
    }
}
