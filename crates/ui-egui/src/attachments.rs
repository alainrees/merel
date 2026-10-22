//! Saving an attachment to the Downloads folder, and the preview of a picture. Nothing is
//! opened or run: the file is written under a name cleaned of paths and control characters,
//! never over an existing one, and a notice says where it went. A picture attachment
//! gets a thumbnail under the message and a window with the full picture, decoded by the same
//! image loader the HTML view uses; other kinds are only saved.

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use merel_mime::Attachment;

use crate::notice::{Notice, file_name, folder_name};
use crate::{MailApp, Tokens};

/// Longest file name written, in characters.
const MAX_NAME: usize = 120;

/// The button after the attachment chips of a message with several, Outlook's "Alle bijlagen
/// opslaan".
pub fn save_all_label() -> String {
    tr("attachments-save-all")
}

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

/// Writes every attachment into `dir`, each under its own name (a taken name gets ` (2)`…).
/// Returns how many were written and the first problem, so the rest still go in.
pub fn save_all(dir: &Path, list: &[Attachment]) -> (usize, Option<String>) {
    let mut saved = 0;
    let mut problem = None;
    for a in list {
        match save_to(dir, &a.name, &a.bytes) {
            Ok(_) => saved += 1,
            Err(e) => {
                problem.get_or_insert_with(|| {
                    crate::i18n::tr_args("file-not-saved", &[("name", a.name.as_str().into()), ("error", e.to_string().into())])
                });
            }
        }
    }
    (saved, problem)
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
        let dir = std::env::temp_dir().join(format!("merel-att-{}", std::process::id()));
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

    #[test]
    fn saving_all_writes_each_file_and_keeps_going_after_a_problem() {
        let dir = std::env::temp_dir().join(format!("merel-att-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, bytes: &[u8]| Attachment {
            name: name.into(),
            mime: "application/octet-stream".into(),
            size: bytes.len(),
            bytes: bytes.into(),
        };
        let (n, problem) = save_all(&dir, &[file("verslag.pdf", b"one"), file("verslag.pdf", b"two"), file("foto.png", b"png")]);
        assert_eq!((n, problem), (3, None));
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["foto.png", "verslag (2).pdf", "verslag.pdf"]);
        let (n, problem) = save_all(&dir.join("bestaat-niet"), &[file("a.txt", b"a"), file("b.txt", b"b")]);
        assert_eq!(n, 0);
        assert!(problem.is_some_and(|p| p.starts_with("a.txt niet opgeslagen: ")), "the first problem is named");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Largest picture Merel decodes for a preview, in bytes; bigger ones are only saved.
pub const MAX_PREVIEW: usize = 8 * 1024 * 1024;

/// Whether an attachment is a picture Merel shows a preview of, as Outlook does: one of
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
    crate::i18n::tr_args("attachment-preview", &[("name", name.into())])
}

/// The notice for a file that went into a folder: its name and the folder's, not the whole
/// path, with a button that shows the file.
pub fn saved(what: &str, path: &Path) -> Notice {
    let dir = path.parent().map(folder_name).unwrap_or_default();
    Notice::success(crate::i18n::tr_args("saved-in", &[("what", what.into()), ("folder", dir.into()), ("file", file_name(path).into())]))
        .revealing(path)
}

impl MailApp {
    /// Saves the attachment under Downloads and says where it went.
    pub fn save_attachment(&mut self, a: &Attachment) {
        self.notice = Some(match downloads_dir() {
            Some(dir) => match save_to(&dir, &a.name, &a.bytes) {
                Ok(path) => saved(&tr("noun-attachment"), &path),
                Err(e) => Notice::error(crate::i18n::tr_args("attachment-not-saved", &[("error", e.to_string().into())])),
            },
            None => Notice::error(tr("attachment-no-downloads")),
        });
    }

    /// Saves every attachment of the message under Downloads and says how it went.
    pub fn save_attachments(&mut self, list: &[Attachment]) {
        self.notice = Some(match downloads_dir() {
            Some(dir) => match save_all(&dir, list) {
                (n, None) => Notice::success(crate::i18n::tr_args("attachments-saved", &[("count", n.into()), ("folder", folder_name(&dir).into())]))
                    .revealing(&dir),
                (0, Some(problem)) => Notice::error(crate::i18n::tr_args("attachments-not-saved", &[("error", problem.into())])),
                (n, Some(problem)) => Notice::warning(crate::i18n::tr_args(
                    "attachments-partly-saved",
                    &[("count", n.into()), ("total", list.len().into()), ("folder", folder_name(&dir).into()), ("error", problem.into())],
                ))
                .revealing(&dir),
            },
            None => Notice::error(tr("attachments-no-downloads")),
        });
    }
}

/// The preview window, while a picture is open: the picture fitted to the window, with
/// "Save to Downloads" and "Close".
pub fn preview_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(p) = app.preview.clone() else { return };
    let mut save = false;
    let mut close = false;
    let screen = ctx.content_rect().size();
    let max = egui::vec2((screen.x - 80.0).clamp(200.0, 1000.0), (screen.y - 160.0).clamp(150.0, 700.0));
    if crate::helder::popup(ctx, t, "attachment-preview", &preview_label(&p.attachment.name), max.x, |ui| {
        let image =
            egui::Image::from_bytes(p.uri.clone(), egui::load::Bytes::Shared(p.attachment.bytes.clone())).max_size(max).fit_to_original_size(1.0);
        ui.add(image);
        ui.label(egui::RichText::new(format!("{}  ({})", p.attachment.name, crate::size_text(p.attachment.size))).size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(crate::helder::primary_text(t, &tr("attachment-save-downloads"))).clicked() {
                save = true;
            }
            if ui.button(tr("close")).clicked() {
                close = true;
            }
        });
    }) {
        close = true;
    }
    if save {
        app.save_attachment(&p.attachment);
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.preview = None;
    }
}
